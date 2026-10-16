//! `disklens` command-line front end.
//!
//!   disklens scan C:\ --top 20 [--method auto|walk|findfirst|mft] [--threads N] [--json]
//!   disklens bench C:\ --runs 3 [--method walk]
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
