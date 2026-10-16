//! DiskLens desktop shell: owns the scan session and exposes it to the UI.
//!
//! Scans run on a background thread writing into a shared `RwLock<Tree>`; a ticker emits
//! `scan-progress` events, and the UI pulls fresh treemap layouts (binary) while the scan runs, so the
//! treemap fills in live. Node ids are only valid for one scan `generation`.

mod encode;
mod system;

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use disklens_core::analysis::{extension_totals, largest_files, largest_folders, ExtTotal, Ranked};
use disklens_core::category::{category_of, category_totals, dominant_categories, ext_categories, CategoryTotal};
use disklens_core::layout::{layout, LayoutOptions};
use disklens_core::scan::{scan, Inaccessible, Method, Progress, ScanOptions};
use disklens_core::tree::{flags, NodeId, Tree, NONE};
use serde::Serialize;
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

#[derive(Default)]
struct Session {
    tree: Arc<RwLock<Tree>>,
    progress: Arc<Progress>,
    generation: u64,
    scanning: bool,
    /// Dominant category per node, computed when a scan completes.
    dominant: Option<Arc<Vec<u8>>>,
    summary: Option<Arc<Summary>>,
}

#[derive(Default)]
struct AppState(Mutex<Session>);

impl AppState {
    fn snapshot(&self) -> (Arc<RwLock<Tree>>, u64, Option<Arc<Vec<u8>>>) {
        let s = self.0.lock().unwrap();
        (s.tree.clone(), s.generation, s.dominant.clone())
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
}

#[tauri::command]
fn list_drives() -> Vec<system::Drive> {
    system::list_drives()
}

#[tauri::command]
fn is_elevated() -> bool {
    system::is_elevated()
}

/// Start scanning `path`. Any running scan is cancelled. Returns the new generation.
#[tauri::command]
fn start_scan(app: AppHandle, state: State<'_, AppState>, path: String, method: Option<String>) -> Result<u64, String> {
    let method: Method = method.as_deref().unwrap_or("auto").parse()?;
    let root_name = disklens_core::scan::root_display_name(std::path::Path::new(&path));
    let (tree, progress, generation) = {
        let mut s = state.0.lock().unwrap();
        s.progress.cancel();
        s.generation += 1;
        s.tree = Arc::new(RwLock::new(Tree::new(&root_name)));
        s.progress = Arc::new(Progress::default());
        s.scanning = true;
        s.dominant = None;
        s.summary = None;
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
        let summary = match result {
            Ok(res) => {
                let t = tree.read().unwrap();
                let ext_cat = ext_categories(&t);
                let dominant = Arc::new(dominant_categories(&t, &ext_cat));
                let summary = Arc::new(Summary {
                    generation,
                    root: t.path(0),
                    method: res.method,
                    fallback_reason: res.fallback_reason,
                    cancelled: res.cancelled,
                    elapsed_ms: res.elapsed.as_millis() as u64,
                    files: t.files[0] as u64,
                    dirs: t.dirs[0] as u64,
                    size: t.size[0],
                    size_on_disk: t.alloc[0],
                    nodes: t.len(),
                    tree_bytes: t.heap_bytes(),
                    inaccessible: res.inaccessible,
                    largest_files: largest_files(&t, 50),
                    largest_folders: largest_folders(&t, 50),
                    extensions: extension_totals(&t).into_iter().take(40).collect(),
                    categories: category_totals(&t, &ext_cat),
                });
                let mut s = st.0.lock().unwrap();
                if s.generation == generation {
                    s.dominant = Some(dominant);
                    s.summary = Some(summary.clone());
                    s.scanning = false;
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
    let (tree, current, dominant) = state.snapshot();
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
    Ok(Response::new(encode::encode(&t, root, &blocks, &ext_cat, dominant.as_deref().map(|v| v.as_slice()))))
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
    let (tree, current, dominant) = state.snapshot();
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
                dominant.as_ref().and_then(|d| d.get(i).copied()).unwrap_or(0)
            } else {
                category_of(&t, &ext_cat, c) as u8
            };
            Row {
                id: c,
                name: t.name(c).to_string(),
                size: t.size[i],
                size_on_disk: t.alloc[i],
                files: t.files[i],
                mtime_ms: filetime_to_ms(t.mtime[i]),
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
    /// Folder to scan right away (first positional argument, or `DISKLENS_SCAN`).
    scan: Option<String>,
    /// After the scan, zoom the treemap to this folder (`--focus <path>`).
    focus: Option<String>,
    /// `--theme light|dark`
    theme: Option<String>,
}

/// Command-line options: `DiskLens.exe [folder] [--focus <path>] [--theme light|dark]`.
#[tauri::command]
fn launch_options() -> LaunchOptions {
    let mut o = LaunchOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--focus" => o.focus = args.next(),
            "--theme" => o.theme = args.next(),
            _ if !a.starts_with('-') && o.scan.is_none() && std::path::Path::new(&a).is_dir() => o.scan = Some(a),
            _ => {}
        }
    }
    if o.scan.is_none() {
        o.scan = std::env::var("DISKLENS_SCAN").ok().filter(|p| std::path::Path::new(p).is_dir());
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
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") {
                apply_backdrop(&w, true);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_drives,
            is_elevated,
            start_scan,
            cancel_scan,
            scan_summary,
            treemap_layout,
            node_info,
            children,
            ancestors,
            find_path,
            launch_options,
            set_backdrop
        ])
        .run(tauri::generate_context!())
        .expect("error while running DiskLens");
}
