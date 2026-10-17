//! HaveSpace desktop shell: owns the scan session and exposes it to the UI.
//!
//! Scans run on a background thread writing into a shared `RwLock<Tree>`; a ticker emits
//! `scan-progress` events, and the UI pulls fresh treemap layouts (binary) while the scan runs, so the
//! treemap fills in live. Node ids are only valid for one scan `generation`.

mod encode;
mod integration;
mod system;

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use havespace_core::actions::{self, RecycleItem};
use havespace_core::cache::{self, CacheMeta};
use havespace_core::diff::{diff, ScanDiff};
use havespace_core::dupes::{self, DupeProgress, DupeReport};
use havespace_core::export;
use havespace_core::analysis::{age_summary, extension_totals, largest_files, largest_folders, newest_mtimes, now_filetime, AgeSummary, ExtTotal, Ranked};
use havespace_core::hogs::{find_hogs, Hog};
use havespace_core::protect::{protection, Protection};
use havespace_core::category::{category_of, category_totals, dominant_categories, ext_categories, CategoryTotal};
use havespace_core::layout::{layout, LayoutOptions};
use havespace_core::scan::{scan, Inaccessible, Method, Progress, ScanOptions};
use havespace_core::tree::{flags, NodeId, Tree, NONE};
use serde::Serialize;
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

#[derive(Default)]
struct Session {
    tree: Arc<RwLock<Tree>>,
    progress: Arc<Progress>,
    generation: u64,
    scanning: bool,
    /// Per-node data for drawing, computed when a scan completes.
    derived: Option<Arc<Derived>>,
    summary: Option<Arc<Summary>>,
    /// How the last scan went; kept so the summary can be rebuilt after deletes.
    meta: Option<ScanMeta>,
    /// Duplicate search of this generation: its progress (and cancel), and its result once done.
    dupe_progress: Arc<DupeProgress>,
    dupes_running: bool,
    dupes: Option<Arc<(DupeQuery, DupeReport)>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DupeQuery {
    root: NodeId,
    root_path: String,
    min_size: u64,
}

#[derive(Clone)]
struct ScanMeta {
    method: Method,
    fallback_reason: Option<String>,
    cancelled: bool,
    elapsed_ms: u64,
    inaccessible: Vec<Inaccessible>,
    /// When the scan finished (Unix ms).
    scanned_at_ms: u64,
    /// Loaded from the scan cache rather than scanned just now.
    from_cache: bool,
    /// What changed since the previous saved scan of the same root, once computed.
    changes: Option<Changes>,
}

/// How many changed items and folders the insights panel lists.
const CHANGES_SHOWN: usize = 50;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Changes {
    /// When the earlier scan finished (Unix ms).
    since_ms: u64,
    /// The earlier scan used a different method (e.g. MFT vs walk), so small differences are expected.
    method_differs: bool,
    #[serde(flatten)]
    diff: ScanDiff,
}

/// Per-node values the treemap and tree list draw with, rebuilt with the summary.
pub(crate) struct Derived {
    /// Category holding most of the bytes inside each folder.
    pub dominant: Vec<u8>,
    /// Newest modification (FILETIME) of a file inside each node.
    pub newest: Vec<u64>,
}

/// Everything the insights panel shows, computed from the live (non-deleted) part of the tree.
fn build_summary(generation: u64, t: &Tree, meta: &ScanMeta) -> (Summary, Derived) {
    let ext_cat = ext_categories(t);
    let derived = Derived { dominant: dominant_categories(t, &ext_cat), newest: newest_mtimes(t) };
    let summary = Summary {
        generation,
        root: t.path(0),
        method: meta.method,
        fallback_reason: meta.fallback_reason.clone(),
        cancelled: meta.cancelled,
        elapsed_ms: meta.elapsed_ms,
        scanned_at_ms: meta.scanned_at_ms,
        from_cache: meta.from_cache,
        files: t.files[0] as u64,
        dirs: t.dirs[0] as u64,
        size: t.size[0],
        size_on_disk: t.alloc[0],
        nodes: t.len(),
        tree_bytes: t.heap_bytes(),
        inaccessible: meta.inaccessible.clone(),
        largest_files: largest_files(t, 50),
        largest_folders: largest_folders(t, 50),
        extensions: extension_totals(t).into_iter().take(40).collect(),
        categories: category_totals(t, &ext_cat),
        hogs: find_hogs(t),
        age: age_summary(t, now_filetime(), 50),
        changes: meta.changes.clone(),
    };
    (summary, derived)
}

fn save_cache(t: &Tree, meta: &ScanMeta) {
    let cm = CacheMeta {
        root: t.path(0),
        scanned_at_ms: meta.scanned_at_ms,
        method: meta.method,
        fallback_reason: meta.fallback_reason.clone(),
        elapsed_ms: meta.elapsed_ms,
        inaccessible: meta.inaccessible.clone(),
        size: t.size[0],
        files: t.files[0] as u64,
        dirs: t.dirs[0] as u64,
    };
    if let Err(e) = cache::save(&cm, t) {
        eprintln!("could not save the scan cache: {e}");
    }
    integration::refresh_jump_list();
}

/// Write the scan cache on a background thread (the tree is read-locked while it is serialized).
fn save_cache_async(tree: Arc<RwLock<Tree>>, meta: ScanMeta) {
    std::thread::spawn(move || save_cache(&tree.read().unwrap(), &meta));
}

/// After a fresh scan, on a background thread: compare it with the previous saved scan of the same root
/// (read before it is overwritten), save the new scan, then add the changes to the insights and emit
/// `summary-updated`.
fn diff_then_save_async(app: AppHandle, generation: u64, tree: Arc<RwLock<Tree>>, meta: ScanMeta) {
    std::thread::spawn(move || {
        let t = tree.read().unwrap();
        let changes = match cache::load(&t.path(0)) {
            Ok((old_meta, old)) => Some(Changes {
                since_ms: old_meta.scanned_at_ms,
                method_differs: old_meta.method != meta.method,
                diff: diff(&old, &t, CHANGES_SHOWN),
            }),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("could not read the previous scan to compare: {e}");
                }
                None
            }
        };
        save_cache(&t, &meta);
        drop(t);
        let Some(changes) = changes else { return };
        let st = app.state::<AppState>();
        let summary = {
            let mut s = st.0.lock().unwrap();
            if s.generation != generation {
                return;
            }
            if let Some(m) = s.meta.as_mut() {
                m.changes = Some(changes.clone());
            }
            let Some(old) = s.summary.as_deref() else { return };
            let summary = Arc::new(Summary { changes: Some(changes), ..old.clone() });
            s.summary = Some(summary.clone());
            summary
        };
        let _ = app.emit("summary-updated", summary.as_ref());
    });
}

impl Session {
    /// Cancel any duplicate search and forget its result (a new scan invalidates node ids).
    fn reset_dupes(&mut self) {
        self.dupe_progress.cancel();
        self.dupe_progress = Arc::new(DupeProgress::default());
        self.dupes_running = false;
        self.dupes = None;
    }
}

#[derive(Default)]
struct AppState(Mutex<Session>);

impl AppState {
    fn snapshot(&self) -> (Arc<RwLock<Tree>>, u64, Option<Arc<Derived>>) {
        let s = self.0.lock().unwrap();
        (s.tree.clone(), s.generation, s.derived.clone())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    generation: u64,
    files: u64,
    dirs: u64,
    bytes: u64,
    elapsed_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    generation: u64,
    root: String,
    method: Method,
    fallback_reason: Option<String>,
    cancelled: bool,
    elapsed_ms: u64,
    scanned_at_ms: u64,
    from_cache: bool,
    files: u64,
    dirs: u64,
    size: u64,
    size_on_disk: u64,
    nodes: usize,
    tree_bytes: usize,
    inaccessible: Vec<Inaccessible>,
    largest_files: Vec<Ranked>,
    largest_folders: Vec<Ranked>,
    extensions: Vec<ExtTotal>,
    categories: Vec<CategoryTotal>,
    hogs: Vec<Hog>,
    age: AgeSummary,
    changes: Option<Changes>,
}

#[tauri::command]
fn list_drives() -> Vec<system::Drive> {
    system::list_drives()
}

#[tauri::command]
fn is_elevated() -> bool {
    system::is_elevated()
}

/// Restart HaveSpace as administrator (UAC prompt) so drive scans can read the MFT directly. `scan` is
/// passed on so the elevated instance starts scanning the same place. Exits this instance on success.
#[tauri::command]
fn restart_as_admin(app: AppHandle, scan: Option<String>, theme: Option<String>) -> Result<(), String> {
    let mut args = Vec::new();
    if let Some(s) = scan {
        args.push(s);
    }
    if let Some(t) = theme {
        args.push("--theme".into());
        args.push(t);
    }
    system::relaunch_elevated(&args).map_err(|e| {
        if e.raw_os_error() == Some(1223) {
            "The administrator prompt was declined.".to_string()
        } else {
            e.to_string()
        }
    })?;
    app.exit(0);
    Ok(())
}

/// Start scanning `path`. Any running scan is cancelled. Returns the new generation.
#[tauri::command]
fn start_scan(app: AppHandle, state: State<'_, AppState>, path: String, method: Option<String>) -> Result<u64, String> {
    let method: Method = method.as_deref().unwrap_or("auto").parse()?;
    let root_name = havespace_core::scan::root_display_name(std::path::Path::new(&path));
    let (tree, progress, generation) = {
        let mut s = state.0.lock().unwrap();
        s.progress.cancel();
        s.generation += 1;
        s.tree = Arc::new(RwLock::new(Tree::new(&root_name)));
        s.progress = Arc::new(Progress::default());
        s.scanning = true;
        s.derived = None;
        s.summary = None;
        s.meta = None;
        s.reset_dupes();
        (s.tree.clone(), s.progress.clone(), s.generation)
    };

    let start = Instant::now();
    // Progress ticker.
    {
        let app = app.clone();
        let progress = progress.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(120));
            let st = app.state::<AppState>();
            let live = {
                let s = st.0.lock().unwrap();
                s.generation == generation && s.scanning
            };
            if !live {
                break;
            }
            let p = progress.snapshot();
            let _ = app.emit(
                "scan-progress",
                ProgressEvent { generation, files: p.files, dirs: p.dirs, bytes: p.bytes, elapsed_ms: start.elapsed().as_millis() as u64 },
            );
        });
    }

    std::thread::spawn(move || {
        let opts = ScanOptions { root: path.into(), method, threads: 0 };
        let result = scan(&opts, &progress, Some(tree.clone()));
        let st = app.state::<AppState>();
        let mut compare_next = None;
        let summary = match result {
            Ok(res) => {
                let meta = ScanMeta {
                    method: res.method,
                    fallback_reason: res.fallback_reason,
                    cancelled: res.cancelled,
                    elapsed_ms: res.elapsed.as_millis() as u64,
                    inaccessible: res.inaccessible,
                    scanned_at_ms: cache::now_ms(),
                    from_cache: false,
                    changes: None,
                };
                let (summary, derived) = build_summary(generation, &tree.read().unwrap(), &meta);
                let summary = Arc::new(summary);
                {
                    let mut s = st.0.lock().unwrap();
                    if s.generation == generation {
                        s.derived = Some(Arc::new(derived));
                        s.summary = Some(summary.clone());
                        s.meta = Some(meta.clone());
                        s.scanning = false;
                    }
                }
                // A cancelled scan is partial: comparing or saving it would be misleading.
                if !meta.cancelled {
                    compare_next = Some(meta);
                }
                Ok(summary)
            }
            Err(e) => {
                let mut s = st.0.lock().unwrap();
                if s.generation == generation {
                    s.scanning = false;
                }
                Err(e.to_string())
            }
        };
        match summary {
            Ok(s) => {
                let _ = app.emit("scan-done", s.as_ref());
                // After `scan-done`, so the `summary-updated` carrying the changes can't arrive first.
                if let Some(meta) = compare_next {
                    diff_then_save_async(app.clone(), generation, tree.clone(), meta);
                }
            }
            Err(e) => {
                let _ = app.emit("scan-error", serde_json::json!({ "generation": generation, "error": e }));
            }
        }
    });
    Ok(generation)
}

#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) {
    state.0.lock().unwrap().progress.cancel();
}

#[tauri::command]
fn scan_summary(state: State<'_, AppState>) -> Option<Summary> {
    state.0.lock().unwrap().summary.as_deref().cloned()
}

/// Treemap layout of `root` for a `width × height` viewport, as the binary format in `encode.rs`.
#[tauri::command]
async fn treemap_layout(state: State<'_, AppState>, generation: u64, root: NodeId, width: f32, height: f32, use_alloc: bool) -> Result<Response, String> {
    let (tree, current, derived) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    let t = tree.read().unwrap();
    if root as usize >= t.len() {
        return Err("no such node".into());
    }
    let ext_cat = ext_categories(&t);
    let opts = LayoutOptions { use_alloc, ..Default::default() };
    let blocks = layout(&t, root, width, height, &opts);
    Ok(Response::new(encode::encode(&t, root, &blocks, &ext_cat, derived.as_deref())))
}

fn filetime_to_ms(ft: u64) -> i64 {
    if ft == 0 {
        0
    } else {
        (ft / 10_000) as i64 - 11_644_473_600_000
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeInfo {
    id: NodeId,
    name: String,
    path: String,
    size: u64,
    size_on_disk: u64,
    files: u32,
    dirs: u32,
    mtime_ms: i64,
    is_dir: bool,
    category: u8,
    parent: Option<NodeId>,
    parent_size: u64,
    link: bool,
    inaccessible: bool,
    hardlink_dup: bool,
}

fn node_info_of(t: &Tree, ext_cat: &[u8], id: NodeId, use_alloc: bool) -> NodeInfo {
    let i = id as usize;
    let p = t.parent[i];
    let size_of = |n: usize| if use_alloc { t.alloc[n] } else { t.size[n] };
    NodeInfo {
        id,
        name: t.name(id).to_string(),
        path: t.path(id),
        size: t.size[i],
        size_on_disk: t.alloc[i],
        files: t.files[i],
        dirs: t.dirs[i],
        mtime_ms: filetime_to_ms(t.mtime[i]),
        is_dir: t.is_dir(id),
        category: category_of(t, ext_cat, id) as u8,
        parent: (p != NONE).then_some(p),
        parent_size: if p != NONE { size_of(p as usize) } else { size_of(i) },
        link: t.has_flag(id, flags::LINK),
        inaccessible: t.has_flag(id, flags::INACCESSIBLE),
        hardlink_dup: t.has_flag(id, flags::HARDLINK_DUP),
    }
}

#[tauri::command]
async fn node_info(state: State<'_, AppState>, generation: u64, id: NodeId, use_alloc: Option<bool>) -> Result<NodeInfo, String> {
    let (tree, current, _) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    let t = tree.read().unwrap();
    if id as usize >= t.len() {
        return Err("no such node".into());
    }
    Ok(node_info_of(&t, &ext_categories(&t), id, use_alloc.unwrap_or(false)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: NodeId,
    name: String,
    size: u64,
    size_on_disk: u64,
    files: u32,
    /// Last modified; for folders, the newest file inside once the scan is done (0 = unknown).
    mtime_ms: i64,
    is_dir: bool,
    has_children: bool,
    category: u8,
    link: bool,
    inaccessible: bool,
}

/// Children of a folder for the tree list, largest first, capped at `limit`.
#[tauri::command]
async fn children(state: State<'_, AppState>, generation: u64, id: NodeId, limit: Option<usize>, use_alloc: Option<bool>) -> Result<Vec<Row>, String> {
    let (tree, current, derived) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    let t = tree.read().unwrap();
    if id as usize >= t.len() {
        return Err("no such node".into());
    }
    let ext_cat = ext_categories(&t);
    let alloc = use_alloc.unwrap_or(false);
    let mut kids: Vec<NodeId> = t.children(id).filter(|&c| !t.has_flag(c, flags::DELETED)).collect();
    kids.sort_unstable_by_key(|&c| std::cmp::Reverse(if alloc { t.alloc[c as usize] } else { t.size[c as usize] }));
    kids.truncate(limit.unwrap_or(500));
    Ok(kids
        .into_iter()
        .map(|c| {
            let i = c as usize;
            let category = if t.is_dir(c) {
                derived.as_ref().and_then(|d| d.dominant.get(i).copied()).unwrap_or(0)
            } else {
                category_of(&t, &ext_cat, c) as u8
            };
            Row {
                id: c,
                name: t.name(c).to_string(),
                size: t.size[i],
                size_on_disk: t.alloc[i],
                files: t.files[i],
                mtime_ms: filetime_to_ms(match derived.as_ref() {
                    Some(d) if t.is_dir(c) => d.newest[i],
                    _ => t.mtime[i],
                }),
                is_dir: t.is_dir(c),
                has_children: t.child_count[i] > 0,
                category,
                link: t.has_flag(c, flags::LINK),
                inaccessible: t.has_flag(c, flags::INACCESSIBLE),
            }
        })
        .collect())
}

#[derive(Serialize)]
struct Crumb {
    id: NodeId,
    name: String,
    size: u64,
}

/// Ancestors of `id` from the root down (inclusive).
#[tauri::command]
async fn ancestors(state: State<'_, AppState>, generation: u64, id: NodeId) -> Result<Vec<Crumb>, String> {
    let (tree, current, _) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    let t = tree.read().unwrap();
    if id as usize >= t.len() {
        return Err("no such node".into());
    }
    let mut out = Vec::new();
    let mut cur = id;
    while cur != NONE {
        out.push(Crumb { id: cur, name: t.name(cur).to_string(), size: t.size[cur as usize] });
        cur = t.parent[cur as usize];
    }
    out.reverse();
    Ok(out)
}

#[derive(Serialize, Default)]
struct LaunchOptions {
    /// Folder to scan right away (first positional argument, or `HAVESPACE_SCAN`).
    scan: Option<String>,
    /// Open the saved scan of `scan` instead of scanning (`--saved`, used by the jump list).
    saved: bool,
    /// After the scan, zoom the treemap to this folder (`--focus <path>`).
    focus: Option<String>,
    /// `--theme light|dark`
    theme: Option<String>,
}

/// Command-line options: `HaveSpace.exe [folder] [--saved] [--focus <path>] [--theme light|dark] [--tray]`.
#[tauri::command]
fn launch_options() -> LaunchOptions {
    let mut o = LaunchOptions::default();
    // Explorer passes a drive root as `"C:\"`, which Windows' argument rules turn into `C:"` (sometimes
    // with trailing spaces): put the backslash back.
    let mut args = std::env::args().skip(1).map(|a| match a.trim_end().strip_suffix('"') {
        Some(rest) => format!("{rest}\\"),
        None => a,
    });
    while let Some(a) = args.next() {
        match a.as_str() {
            "--focus" => o.focus = args.next(),
            "--theme" => o.theme = args.next(),
            "--saved" => o.saved = true,
            _ if !a.starts_with('-') && o.scan.is_none() && std::path::Path::new(&a).is_dir() => o.scan = Some(a),
            _ => {}
        }
    }
    if o.scan.is_none() {
        o.scan = std::env::var("HAVESPACE_SCAN").ok().filter(|p| std::path::Path::new(p).is_dir());
    }
    o
}

/// Resolve a full path (e.g. from the insights list) to a node id in the current tree.
#[tauri::command]
async fn find_path(state: State<'_, AppState>, generation: u64, path: String) -> Result<Option<NodeId>, String> {
    let (tree, current, _) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    let t = tree.read().unwrap();
    let root = t.path(0);
    let Some(rest) = path.get(..root.len()).filter(|p| p.eq_ignore_ascii_case(&root)).map(|_| &path[root.len()..]) else {
        return Ok(None);
    };
    let mut cur = 0;
    for part in rest.split('\\').filter(|s| !s.is_empty()) {
        match t.child_by_name(cur, part) {
            Some(c) => cur = c,
            None => return Ok(None),
        }
    }
    Ok(Some(cur))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionInfo {
    id: NodeId,
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    files: u32,
    protection: Protection,
    /// Why "Send to Recycle Bin" is unavailable (protected, no bin, too big, scanning...), if it is.
    recycle_blocked: Option<String>,
}

fn recycle_size(t: &Tree, id: NodeId) -> u64 {
    t.size[id as usize].max(t.alloc[id as usize])
}

/// What can be done with a node; drives the context menu and the confirmation dialog.
#[tauri::command]
async fn action_info(state: State<'_, AppState>, generation: u64, id: NodeId) -> Result<ActionInfo, String> {
    let scanning = state.0.lock().unwrap().scanning;
    let (tree, current, _) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    let t = tree.read().unwrap();
    if id as usize >= t.len() {
        return Err("no such node".into());
    }
    let path = t.path(id);
    let prot = protection(&path);
    let size = recycle_size(&t, id);
    let recycle_blocked = if scanning {
        Some("Wait for the scan to finish.".to_string())
    } else if id == 0 {
        Some("This is the folder you scanned.".to_string())
    } else if let Protection::Blocked(r) = &prot {
        Some(r.to_string())
    } else if t.has_flag(id, flags::DELETED) {
        Some("Already sent to the Recycle Bin.".to_string())
    } else {
        match actions::recycle_bin_capacity(&path) {
            None => Some(actions::NO_RECYCLE_BIN.to_string()),
            Some(cap) if size as f64 > cap as f64 * 0.98 => Some(actions::TOO_BIG_FOR_BIN.to_string()),
            Some(_) => None,
        }
    };
    Ok(ActionInfo { id, name: t.name(id).to_string(), path, is_dir: t.is_dir(id), size: t.size[id as usize], files: t.files[id as usize], protection: prot, recycle_blocked })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecycleResponse {
    recycled: Vec<NodeId>,
    failed: Vec<(String, String)>,
    permanently_deleted: Vec<String>,
    /// Logical bytes removed from the drive.
    freed: u64,
}

/// Send nodes to the Recycle Bin (the UI has already asked the user to confirm), then update every
/// total in place: no rescan.
#[tauri::command]
async fn recycle_nodes(app: AppHandle, state: State<'_, AppState>, generation: u64, ids: Vec<NodeId>) -> Result<RecycleResponse, String> {
    if state.0.lock().unwrap().scanning {
        return Err("Wait for the scan to finish.".into());
    }
    let (tree, current, _) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    // Resolve and vet the items under a read lock.
    let mut items = Vec::new();
    let mut by_path = Vec::new();
    let mut failed = Vec::new();
    {
        let t = tree.read().unwrap();
        for id in ids {
            if id == 0 || id as usize >= t.len() || t.has_flag(id, flags::DELETED) {
                continue;
            }
            let path = t.path(id);
            if let Protection::Blocked(r) = protection(&path) {
                failed.push((path, r.to_string()));
                continue;
            }
            by_path.push((path.clone(), id));
            items.push(RecycleItem { path, size: recycle_size(&t, id) });
        }
    }
    let out = tauri::async_runtime::spawn_blocking(move || actions::recycle(items)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    failed.extend(out.failed);

    // Update totals for everything that is gone (recycled, or, should the tripwire ever fire, deleted).
    let gone: Vec<NodeId> = out
        .recycled
        .iter()
        .chain(&out.permanently_deleted)
        .filter_map(|p| by_path.iter().find(|(bp, _)| bp.eq_ignore_ascii_case(p)).map(|(_, id)| *id))
        .collect();
    let mut freed = 0;
    {
        let mut t = tree.write().unwrap();
        for &id in &gone {
            if !t.has_flag(id, flags::DELETED) {
                freed += t.size[id as usize];
                t.mark_deleted(id);
            }
        }
    }
    let recycled: Vec<NodeId> = out.recycled.iter().filter_map(|p| by_path.iter().find(|(bp, _)| bp.eq_ignore_ascii_case(p)).map(|(_, id)| *id)).collect();

    if !gone.is_empty() {
        // Rebuild insights from the updated tree and let the UI refresh.
        let meta = state.0.lock().unwrap().meta.clone();
        if let Some(meta) = meta {
            let (summary, derived) = build_summary(generation, &tree.read().unwrap(), &meta);
            let summary = Arc::new(summary);
            {
                let mut s = state.0.lock().unwrap();
                if s.generation == generation {
                    s.summary = Some(summary.clone());
                    s.derived = Some(Arc::new(derived));
                }
            }
            let _ = app.emit("summary-updated", summary.as_ref());
            // Keep the cache in step with what is on disk now.
            if !meta.cancelled {
                save_cache_async(tree.clone(), meta);
            }
        }
    }
    Ok(RecycleResponse { recycled, failed, permanently_deleted: out.permanently_deleted, freed })
}

/// How many duplicate sets the UI gets (the totals still cover every set).
const DUPE_SETS_SHOWN: usize = 300;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DupeView {
    generation: u64,
    #[serde(flatten)]
    query: DupeQuery,
    total_sets: usize,
    #[serde(flatten)]
    report: DupeReport,
}

/// The finished search without items since sent to the Recycle Bin, capped for the UI.
fn dupe_view(generation: u64, t: &Tree, done: &(DupeQuery, DupeReport)) -> DupeView {
    let mut report = dupes::without_deleted(&done.1, t);
    let total_sets = report.sets.len();
    report.sets.truncate(DUPE_SETS_SHOWN);
    DupeView { generation, query: done.0.clone(), total_sets, report }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DupeProgressEvent {
    generation: u64,
    phase: u8,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
}

/// Look for duplicate files under `root` in the background. Emits `dupes-progress` while reading and
/// `dupes-done` (a `DupeView`) at the end. Any earlier search is cancelled.
#[tauri::command]
fn start_duplicates(app: AppHandle, state: State<'_, AppState>, generation: u64, root: NodeId, min_size: u64) -> Result<(), String> {
    let (tree, progress) = {
        let mut s = state.0.lock().unwrap();
        if s.generation != generation {
            return Err("stale generation".into());
        }
        if s.scanning {
            return Err("Wait for the scan to finish.".into());
        }
        s.reset_dupes();
        s.dupes_running = true;
        (s.tree.clone(), s.dupe_progress.clone())
    };
    // Ticker.
    {
        let app = app.clone();
        let progress = progress.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(200));
            let live = {
                let st = app.state::<AppState>();
                let s = st.0.lock().unwrap();
                s.generation == generation && s.dupes_running && Arc::ptr_eq(&s.dupe_progress, &progress)
            };
            if !live {
                break;
            }
            let ld = |a: &std::sync::atomic::AtomicU64| a.load(std::sync::atomic::Ordering::Relaxed);
            let _ = app.emit(
                "dupes-progress",
                DupeProgressEvent {
                    generation,
                    phase: progress.phase.load(std::sync::atomic::Ordering::Relaxed),
                    files_done: ld(&progress.files_done),
                    files_total: ld(&progress.files_total),
                    bytes_done: ld(&progress.bytes_done),
                    bytes_total: ld(&progress.bytes_total),
                },
            );
        });
    }
    std::thread::spawn(move || {
        // Only the candidate list needs the tree; reading files happens without the lock, so
        // recycling and layouts are never blocked by a long search.
        let (cands, query) = {
            let t = tree.read().unwrap();
            if root as usize >= t.len() {
                return;
            }
            (dupes::candidates(&t, root, min_size), DupeQuery { root, root_path: t.path(root), min_size })
        };
        let report = dupes::find(cands, 4, &progress);
        let st = app.state::<AppState>();
        let done = Arc::new((query, report));
        {
            let mut s = st.0.lock().unwrap();
            if s.generation != generation || !Arc::ptr_eq(&s.dupe_progress, &progress) {
                return;
            }
            s.dupes_running = false;
            s.dupes = Some(done.clone());
        }
        let view = dupe_view(generation, &tree.read().unwrap(), &done);
        let _ = app.emit("dupes-done", view);
    });
    Ok(())
}

#[tauri::command]
fn cancel_duplicates(state: State<'_, AppState>) {
    state.0.lock().unwrap().dupe_progress.cancel();
}

/// The last finished duplicate search, re-filtered for anything recycled since.
#[tauri::command]
async fn duplicates(state: State<'_, AppState>, generation: u64) -> Result<Option<DupeView>, String> {
    let (tree, done) = {
        let s = state.0.lock().unwrap();
        if s.generation != generation {
            return Err("stale generation".into());
        }
        (s.tree.clone(), s.dupes.clone())
    };
    let Some(done) = done else { return Ok(None) };
    let t = tree.read().unwrap();
    Ok(Some(dupe_view(generation, &t, &done)))
}

/// Cached scans, newest first (for the drive picker).
#[tauri::command]
fn list_caches() -> Vec<CacheMeta> {
    cache::list()
}

/// Open the cached scan of `root` instantly: becomes the current session, then `scan-done` is emitted
/// exactly as for a fresh scan (with `fromCache` set). Returns the new generation.
#[tauri::command]
async fn open_cache(app: AppHandle, state: State<'_, AppState>, root: String) -> Result<u64, String> {
    let (cm, tree) = tauri::async_runtime::spawn_blocking(move || cache::load(&root)).await.map_err(|e| e.to_string())?.map_err(|e| format!("Couldn't open the cached scan: {e}"))?;
    let meta = ScanMeta {
        method: cm.method,
        fallback_reason: cm.fallback_reason,
        cancelled: false,
        elapsed_ms: cm.elapsed_ms,
        inaccessible: cm.inaccessible,
        scanned_at_ms: cm.scanned_at_ms,
        from_cache: true,
        changes: None,
    };
    let tree = Arc::new(RwLock::new(tree));
    let generation = {
        let mut s = state.0.lock().unwrap();
        s.progress.cancel();
        s.generation += 1;
        s.tree = tree.clone();
        s.progress = Arc::new(Progress::default());
        s.scanning = false;
        s.derived = None;
        s.summary = None;
        s.meta = Some(meta.clone());
        s.reset_dupes();
        s.generation
    };
    // Build insights off the async runtime, then announce the "scan" as done.
    let app2 = app.clone();
    std::thread::spawn(move || {
        let (summary, derived) = build_summary(generation, &tree.read().unwrap(), &meta);
        let summary = Arc::new(summary);
        let st = app2.state::<AppState>();
        {
            let mut s = st.0.lock().unwrap();
            if s.generation != generation {
                return;
            }
            s.derived = Some(Arc::new(derived));
            s.summary = Some(summary.clone());
        }
        let _ = app2.emit("scan-done", summary.as_ref());
    });
    Ok(generation)
}

/// Export the subtree under `id` to `path` (the UI asks the user where with a save dialog).
#[tauri::command]
async fn export_tree(state: State<'_, AppState>, generation: u64, id: NodeId, format: String, folders_only: bool, max_depth: Option<u32>, path: String) -> Result<export::Stats, String> {
    let (tree, current, _) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    let format: export::Format = format.parse()?;
    tauri::async_runtime::spawn_blocking(move || -> std::io::Result<export::Stats> {
        use std::io::Write;
        let t = tree.read().unwrap();
        let f = std::fs::File::create(&path)?;
        let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
        let s = export::export(&t, export::Options { format, root: id, folders_only, max_depth }, &mut w)?;
        w.flush()?;
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Export failed: {e}"))
}

/// Open Explorer with the item selected.
#[tauri::command]
async fn reveal_node(state: State<'_, AppState>, generation: u64, id: NodeId) -> Result<(), String> {
    let (tree, current, _) = state.snapshot();
    if generation != current {
        return Err("stale generation".into());
    }
    let path = {
        let t = tree.read().unwrap();
        if id as usize >= t.len() {
            return Err("no such node".into());
        }
        t.path(id)
    };
    tauri::async_runtime::spawn_blocking(move || actions::reveal_in_explorer(path)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

#[tauri::command]
fn get_integration(integ: State<'_, integration::Integration>) -> integration::Info {
    integ.info()
}

#[tauri::command]
async fn set_integration(app: AppHandle, settings: integration::Settings) -> Result<integration::Info, String> {
    tauri::async_runtime::spawn_blocking(move || integration::apply(&app, settings)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn test_notification(app: AppHandle) {
    integration::test_notification(&app);
}

/// Re-apply the Mica backdrop for the current theme.
#[tauri::command]
fn set_backdrop(window: WebviewWindow, dark: bool) {
    apply_backdrop(&window, dark);
}

fn apply_backdrop(window: &WebviewWindow, dark: bool) {
    #[cfg(windows)]
    {
        // Mica on Windows 11; fall back to Acrylic on Windows 10.
        if window_vibrancy::apply_mica(window, Some(dark)).is_err() {
            let tint = if dark { (16, 18, 32, 160) } else { (240, 242, 248, 160) };
            let _ = window_vibrancy::apply_acrylic(window, Some(tint));
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .manage(integration::Integration::new())
        .setup(|app| {
            integration::init(app.handle());
            if let Some(w) = app.get_webview_window("main") {
                apply_backdrop(&w, true);
                // The window starts hidden (tauri.conf.json) so `--tray` at sign-in shows nothing.
                let integ = app.state::<integration::Integration>();
                if !(integ.start_hidden && integ.tray_active()) {
                    let _ = w.show();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // With the tray on, closing the window keeps HaveSpace running there.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.app_handle().state::<integration::Integration>().tray_active() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_drives,
            is_elevated,
            restart_as_admin,
            start_scan,
            cancel_scan,
            scan_summary,
            treemap_layout,
            node_info,
            children,
            ancestors,
            find_path,
            launch_options,
            action_info,
            recycle_nodes,
            reveal_node,
            list_caches,
            open_cache,
            export_tree,
            start_duplicates,
            cancel_duplicates,
            duplicates,
            get_integration,
            set_integration,
            test_notification,
            set_backdrop
        ])
        .run(tauri::generate_context!())
        .expect("error while running HaveSpace");
}
