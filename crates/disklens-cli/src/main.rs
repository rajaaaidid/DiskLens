//! `disklens` command-line front end.
//!
//!   disklens scan C:\ --top 20 [--method auto|walk|findfirst|mft] [--threads N] [--json]
//!   disklens bench C:\ --runs 3 [--method walk]
//!   disklens diff C:\ [--top 20] [--save] [--against old.dlc] [--json]
//!   disklens fixture D:\tmp\fake --files 1000000 --depth 12

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use disklens_core::analysis::{extension_totals, largest_files, largest_folders};
use disklens_core::{format_bytes, scan, Method, Progress, ScanOptions};

#[derive(Parser)]
#[command(name = "disklens", version, about = "Fast disk space analyzer (scanner engine CLI)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scan a drive or folder and print a summary.
    Scan {
        path: PathBuf,
        /// How many largest files/folders to list.
        #[arg(long, default_value_t = 20)]
        top: usize,
        #[arg(long, default_value = "auto")]
        method: Method,
        /// Worker threads (0 = auto).
        #[arg(long, default_value_t = 0)]
        threads: usize,
        /// Print machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Hide the live progress line.
        #[arg(long)]
        quiet: bool,
        /// After scanning, list the children of this sub-path (relative to the scan root).
        #[arg(long)]
        show: Option<String>,
    },
    /// Scan repeatedly and report timing and memory.
    Bench {
        path: PathBuf,
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[arg(long, default_value = "auto")]
        method: Method,
        #[arg(long, default_value_t = 0)]
        threads: usize,
    },
    /// Scan a drive with the MFT reader and with the directory walk, and compare the results
    /// (needs administrator rights).
    Compare { path: PathBuf },
    /// Scan, then export the result as CSV or JSON.
    Export {
        path: PathBuf,
        #[arg(long, default_value = "csv")]
        format: disklens_core::export::Format,
        /// Output file (default: stdout).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Folders only (no files).
        #[arg(long)]
        folders_only: bool,
        /// Levels below the root to include.
        #[arg(long)]
        depth: Option<u32>,
        #[arg(long, default_value = "auto")]
        method: Method,
    },
    /// Scan, then show what changed since the saved scan of the same drive or folder (the app saves
    /// one after every scan).
    Diff {
        path: PathBuf,
        /// How many changed items and folders to list.
        #[arg(long, default_value_t = 20)]
        top: usize,
        /// Compare against this cache file instead of the saved scan of `path`.
        #[arg(long)]
        against: Option<PathBuf>,
        /// Save this scan as the new baseline for the next diff (and for the app's *Recent scans*).
        #[arg(long)]
        save: bool,
        #[arg(long, default_value = "auto")]
        method: Method,
        /// Print machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Generate a fake directory tree for testing and benchmarks.
    Fixture {
        path: PathBuf,
        #[arg(long, default_value_t = 100_000)]
        files: u64,
        #[arg(long, default_value_t = 10)]
        depth: u32,
        /// Largest file size in bytes.
        #[arg(long, default_value_t = 256 * 1024 * 1024)]
        max_file: u64,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.cmd {
        Cmd::Scan { path, top, method, threads, json, quiet, show } => run_scan(path, top, method, threads, json, quiet, show),
        Cmd::Bench { path, runs, method, threads } => run_bench(path, runs, method, threads),
        Cmd::Compare { path } => run_compare(path),
        Cmd::Export { path, format, out, folders_only, depth, method } => run_export(path, format, out, folders_only, depth, method),
        Cmd::Diff { path, top, against, save, method, json } => run_diff(path, top, against, save, method, json),
        Cmd::Fixture { path, files, depth, max_file, seed } => {
            let t = Instant::now();
            match disklens_fixtures::generate(&path, files, depth, max_file, seed) {
                Ok(s) => {
                    println!("created {} files in {} folders, {} logical, in {:.1}s", s.files, s.dirs, format_bytes(s.bytes), t.elapsed().as_secs_f64());
                    0
                }
                Err(e) => {
                    eprintln!("fixture failed: {e}");
                    1
                }
            }
        }
    };
    std::process::exit(code);
}

fn run_scan(path: PathBuf, top: usize, method: Method, threads: usize, json: bool, quiet: bool, show: Option<String>) -> i32 {
    let progress = Arc::new(Progress::default());
    let opts = ScanOptions { root: path, method, threads };

    // Ctrl+C cancels the scan cleanly (partial results are still printed).
    {
        let p = progress.clone();
        ctrlc_cancel(move || p.cancel());
    }

    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ticker = (!quiet && !json).then(|| {
        let p = progress.clone();
        let done = done.clone();
        let start = Instant::now();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(200));
            if done.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let s = p.snapshot();
            eprint!(
                "\r  {:>10} files  {:>8} folders  {:>10}  {:>6.1}s ",
                s.files,
                s.dirs,
                format_bytes(s.bytes),
                start.elapsed().as_secs_f64()
            );
        })
    });

    let res = scan(&opts, &progress, None);
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(t) = ticker {
        let _ = t.join();
        eprint!("\r{:70}\r", "");
    }
    let res = match res {
        Ok(r) => r,
        Err(e) => {
            eprintln!("scan failed: {e}");
            return 1;
        }
    };
    let tree = res.tree.read().unwrap();
    if let Some(sub) = show {
        let mut cur = 0;
        for part in sub.split(['\\', '/']).filter(|s| !s.is_empty()) {
            match tree.child_by_name(cur, part) {
                Some(c) => cur = c,
                None => {
                    eprintln!("{sub}: '{part}' not found under {}", tree.path(cur));
                    return 1;
                }
            }
        }
        use disklens_core::tree::flags as f;
        println!("{}  {}  {} files  {} folders  flags {:#04x}", tree.path(cur), format_bytes(tree.size[cur as usize]), tree.files[cur as usize], tree.dirs[cur as usize], tree.flags[cur as usize]);
        let mut kids: Vec<_> = tree.children(cur).collect();
        kids.sort_by_key(|&c| std::cmp::Reverse(tree.size[c as usize]));
        for c in kids {
            let fl = tree.flags[c as usize];
            let mut tags = Vec::new();
            for (bit, name) in [(f::LINK, "link"), (f::INACCESSIBLE, "denied"), (f::HARDLINK_DUP, "hardlink-dup"), (f::REPARSE_OTHER, "reparse"), (f::SCANNED, "scanned")] {
                if fl & bit != 0 {
                    tags.push(name);
                }
            }
            println!("  {:>10}  {:>8} files  {}{}  [{}]", format_bytes(tree.size[c as usize]), tree.files[c as usize], tree.name(c), if tree.is_dir(c) { "\\" } else { "" }, tags.join(","));
        }
        return 0;
    }
    let files = largest_files(&tree, top);
    let folders = largest_folders(&tree, top);
    let exts = extension_totals(&tree);
    let mem = peak_working_set();

    if json {
        let v = serde_json::json!({
            "root": tree.path(0),
            "method": res.method,
            "fallback_reason": res.fallback_reason,
            "cancelled": res.cancelled,
            "elapsed_ms": res.elapsed.as_millis() as u64,
            "files": tree.files[0],
            "folders": tree.dirs[0],
            "nodes": tree.len(),
            "size": tree.size[0],
            "size_on_disk": tree.alloc[0],
            "tree_bytes": tree.heap_bytes(),
            "peak_working_set": mem,
            "inaccessible": res.inaccessible,
            "largest_files": files,
            "largest_folders": folders,
            "extensions": exts.iter().take(top).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return 0;
    }

    println!("DiskLens scan of {}", tree.path(0));
    println!(
        "  method        {:?}{}",
        res.method,
        res.fallback_reason.as_ref().map(|r| format!("  (MFT unavailable: {r})")).unwrap_or_default()
    );
    println!("  time          {:.2} s{}", res.elapsed.as_secs_f64(), if res.cancelled { "  (cancelled)" } else { "" });
    println!("  files         {}", tree.files[0]);
    println!("  folders       {}", tree.dirs[0]);
    println!("  size          {}  ({} bytes)", format_bytes(tree.size[0]), tree.size[0]);
    println!("  size on disk  {}", format_bytes(tree.alloc[0]));
    println!("  tree memory   {}  for {} nodes  (process peak {})", format_bytes(tree.heap_bytes() as u64), tree.len(), format_bytes(mem));
    let rate = tree.files[0] as f64 / res.elapsed.as_secs_f64().max(1e-6);
    println!("  throughput    {:.0} files/s", rate);
    if !res.inaccessible.is_empty() {
        println!("  inaccessible  {} folders", res.inaccessible.len());
        for i in res.inaccessible.iter().take(8) {
            println!("                {}  ({})", i.path, i.error);
        }
        if res.inaccessible.len() > 8 {
            println!("                … and {} more", res.inaccessible.len() - 8);
        }
    }
    println!("\nLargest files");
    for (k, f) in files.iter().enumerate() {
        println!("  {:>3}. {:>10}  {}", k + 1, format_bytes(f.size), f.path);
    }
    println!("\nLargest folders (by their own files)");
    for (k, f) in folders.iter().enumerate() {
        println!("  {:>3}. {:>10}  {}", k + 1, format_bytes(f.size), f.path);
    }
    println!("\nTop extensions");
    for e in exts.iter().take(top.min(12)) {
        println!("  {:>10}  {:>9} files  .{}", format_bytes(e.size), e.files, e.ext);
    }
    0
}

fn run_bench(path: PathBuf, runs: usize, method: Method, threads: usize) -> i32 {
    let mut times = Vec::new();
    for run in 1..=runs {
        let progress = Progress::default();
        let opts = ScanOptions { root: path.clone(), method, threads };
        match scan(&opts, &progress, None) {
            Ok(r) => {
                let t = r.tree.read().unwrap();
                println!(
                    "run {run}: {:>6.2} s  {:?}  {} files  {} folders  {}  tree {}  peak RSS {}",
                    r.elapsed.as_secs_f64(),
                    r.method,
                    t.files[0],
                    t.dirs[0],
                    format_bytes(t.size[0]),
                    format_bytes(t.heap_bytes() as u64),
                    format_bytes(peak_working_set())
                );
                times.push(r.elapsed.as_secs_f64());
            }
            Err(e) => {
                eprintln!("run {run} failed: {e}");
                return 1;
            }
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("best {:.2} s, median {:.2} s (run 1 is the coldest)", times[0], times[times.len() / 2]);
    0
}

fn run_export(path: PathBuf, format: disklens_core::export::Format, out: Option<PathBuf>, folders_only: bool, depth: Option<u32>, method: Method) -> i32 {
    use std::io::Write;
    let progress = Progress::default();
    let res = match scan(&ScanOptions { root: path, method, threads: 0 }, &progress, None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("scan failed: {e}");
            return 1;
        }
    };
    let tree = res.tree.read().unwrap();
    let opts = disklens_core::export::Options { format, root: 0, folders_only, max_depth: depth };
    let t0 = Instant::now();
    let result = match &out {
        Some(p) => std::fs::File::create(p).and_then(|f| {
            let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
            let s = disklens_core::export::export(&tree, opts, &mut w)?;
            w.flush()?;
            Ok(s)
        }),
        None => {
            let stdout = std::io::stdout();
            let mut w = std::io::BufWriter::new(stdout.lock());
            disklens_core::export::export(&tree, opts, &mut w).and_then(|s| w.flush().map(|_| s))
        }
    };
    match result {
        Ok(s) => {
            eprintln!("exported {} items in {:.2} s (scan {:.2} s)", s.items, t0.elapsed().as_secs_f64(), res.elapsed.as_secs_f64());
            0
        }
        Err(e) => {
            eprintln!("export failed: {e}");
            1
        }
    }
}

fn run_diff(path: PathBuf, top: usize, against: Option<PathBuf>, save: bool, method: Method, json: bool) -> i32 {
    use disklens_core::cache::{self, CacheMeta};
    use disklens_core::diff::{diff, ChangeKind};

    let root = disklens_core::scan::root_display_name(&path);
    // Read the baseline before scanning: `--save` replaces it.
    let baseline = match &against {
        Some(f) => cache::load_file(f).map(Some),
        None => match cache::load(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            r => r.map(Some),
        },
    };
    let baseline = match baseline {
        Ok(b) => b,
        Err(e) => {
            eprintln!("could not read the saved scan: {e}");
            return 1;
        }
    };
    if baseline.is_none() && !save {
        eprintln!("No saved scan of {root} yet. Scan it in the app, or run this again with --save to keep this scan as the baseline.");
        return 1;
    }

    let progress = Progress::default();
    let res = match scan(&ScanOptions { root: path, method, threads: 0 }, &progress, None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("scan failed: {e}");
            return 1;
        }
    };
    if res.cancelled {
        eprintln!("scan cancelled; nothing compared or saved");
        return 1;
    }
    let tree = res.tree.read().unwrap();
    let scanned_at_ms = cache::now_ms();
    if save {
        let meta = CacheMeta {
            root: tree.path(0),
            scanned_at_ms,
            method: res.method,
            fallback_reason: res.fallback_reason.clone(),
            elapsed_ms: res.elapsed.as_millis() as u64,
            inaccessible: res.inaccessible.clone(),
            size: tree.size[0],
            files: tree.files[0] as u64,
            dirs: tree.dirs[0] as u64,
        };
        if let Err(e) = cache::save(&meta, &tree) {
            eprintln!("could not save the scan: {e}");
            return 1;
        }
    }
    let Some((old_meta, old)) = baseline else {
        eprintln!("Saved this scan of {root} as the baseline. Run the same command later to see what changed.");
        return 0;
    };

    let d = diff(&old, &tree, top);
    if json {
        let v = serde_json::json!({
            "root": tree.path(0),
            "since_ms": old_meta.scanned_at_ms,
            "method_before": old_meta.method,
            "method_after": res.method,
            "diff": d,
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return 0;
    }

    let signed = |v: i64| format!("{}{}", if v < 0 { "-" } else { "+" }, format_bytes(v.unsigned_abs()));
    let ago = format_ago(scanned_at_ms.saturating_sub(old_meta.scanned_at_ms));
    println!("DiskLens changes in {} since the saved scan ({ago})", tree.path(0));
    println!("  size          {} -> {}  ({})", format_bytes(d.before), format_bytes(d.after), signed(d.after as i64 - d.before as i64));
    println!("  files         {} -> {}", d.files_before, d.files_after);
    println!("  grew          {}  ({} new, {} changed)", format_bytes(d.grew), d.added, d.changed);
    println!("  freed         {}  ({} gone)", format_bytes(d.shrank), d.removed);
    if old_meta.method != res.method {
        println!("  note          the saved scan used {:?} and this one {:?}; small differences are expected", old_meta.method, res.method);
    }
    if d.items.is_empty() {
        println!("\nNothing changed.");
        return 0;
    }
    println!("\nFolders that changed most (changes directly inside them)");
    for (k, c) in d.folders.iter().enumerate() {
        println!("  {:>3}. {:>11}  {}", k + 1, signed(c.delta), c.path);
    }
    println!("\nBiggest changes");
    for (k, c) in d.items.iter().enumerate() {
        let tag = match c.kind {
            ChangeKind::Added => "new",
            ChangeKind::Removed => "gone",
            ChangeKind::Grew => "grew",
            ChangeKind::Shrank => "shrank",
        };
        println!("  {:>3}. {:>11}  {:<6} {}{}", k + 1, signed(c.delta), tag, c.path, if c.is_dir { "\\" } else { "" });
    }
    0
}

fn format_ago(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        0 => "just now".into(),
        1..=59 => format!("{m} min ago"),
        60..=2879 => format!("{} h ago", m / 60),
        _ => format!("{} days ago", m / 1440),
    }
}

fn run_compare(path: PathBuf) -> i32 {
    let mut results = Vec::new();
    for method in [Method::Mft, Method::Walk] {
        let progress = Progress::default();
        match scan(&ScanOptions { root: path.clone(), method, threads: 0 }, &progress, None) {
            Ok(r) => {
                let t = std::mem::take(&mut *r.tree.write().unwrap());
                println!(
                    "{:<5} {:>7.2} s  {:>9} files  {:>8} folders  {:>10}  on disk {:>10}  inaccessible {}  peak RSS {}",
                    format!("{method:?}"),
                    r.elapsed.as_secs_f64(),
                    t.files[0],
                    t.dirs[0],
                    format_bytes(t.size[0]),
                    format_bytes(t.alloc[0]),
                    r.inaccessible.len(),
                    format_bytes(peak_working_set())
                );
                results.push(t);
            }
            Err(e) => {
                eprintln!("{method:?} failed: {e}");
                return 1;
            }
        }
    }
    let (mft, walk) = (&results[0], &results[1]);
    println!("\nTop-level folders (MFT vs walk):");
    let mut rows: Vec<_> = mft.children(0).map(|c| (mft.name(c).to_string(), mft.size[c as usize], walk.child_by_name(0, mft.name(c)).map(|w| walk.size[w as usize]))).collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, m, w) in rows.iter().take(25) {
        let diff = match w {
            Some(w) if *w == *m => "same".to_string(),
            Some(0) => "walk could not read it".to_string(),
            Some(w) => format!("{:+.2}%", (*m as f64 - *w as f64) / *w as f64 * 100.0),
            None if name.starts_with('$') => "NTFS metadata (MFT only)".to_string(),
            None => "only in MFT".to_string(),
        };
        println!("  {:<36} {:>10}  {:>10}  {}", name, format_bytes(*m), w.map(format_bytes).unwrap_or_else(|| "-".into()), diff);
    }
    0
}

#[cfg(windows)]
fn peak_working_set() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } != 0 {
        c.PeakWorkingSetSize as u64
    } else {
        0
    }
}

#[cfg(not(windows))]
fn peak_working_set() -> u64 {
    0
}

#[cfg(windows)]
fn ctrlc_cancel(f: impl Fn() + Send + Sync + 'static) {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::BOOL;
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    static HANDLER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
    unsafe extern "system" fn on_ctrl(_: u32) -> BOOL {
        if let Some(h) = HANDLER.get() {
            h();
        }
        1
    }
    let _ = HANDLER.set(Box::new(f));
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
}

#[cfg(not(windows))]
fn ctrlc_cancel(_f: impl Fn() + Send + Sync + 'static) {}
