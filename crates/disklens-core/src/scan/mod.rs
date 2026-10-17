//! Scanning: a parallel directory walk (always available) and an NTFS MFT reader (admin + NTFS).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::tree::Tree;

pub mod enumerate;
pub mod mft;
mod walk;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// MFT if possible, otherwise the directory walk.
    Auto,
    /// Parallel walk using `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)`.
    Walk,
    /// Parallel walk using `FindFirstFileExW(FindExInfoBasic, LARGE_FETCH)` (no hard-link detection).
    FindFirst,
    /// Read the NTFS Master File Table directly (requires admin).
    Mft,
}

impl std::str::FromStr for Method {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Method::Auto),
            "walk" => Ok(Method::Walk),
            "findfirst" | "find" => Ok(Method::FindFirst),
            "mft" => Ok(Method::Mft),
            _ => Err(format!("unknown method '{s}' (auto, walk, findfirst, mft)")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub root: PathBuf,
    pub method: Method,
    /// Worker threads for the walk. 0 = automatic.
    pub threads: usize,
}

impl ScanOptions {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ScanOptions { root: root.into(), method: Method::Auto, threads: 0 }
    }
}

/// Live counters, readable from any thread while a scan runs.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            files: self.files.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProgressSnapshot {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inaccessible {
    pub path: String,
    pub error: String,
}

pub struct ScanResult {
    pub tree: Arc<RwLock<Tree>>,
    pub method: Method,
    pub elapsed: Duration,
    pub inaccessible: Vec<Inaccessible>,
    pub cancelled: bool,
    /// Why the MFT path was not used, if it was attempted.
    pub fallback_reason: Option<String>,
}

/// Normalise a user path to the tree root name: `C:` / `c:\` -> `C:\`, folders keep their full path.
pub fn root_display_name(root: &Path) -> String {
    let s = root.to_string_lossy().replace('/', "\\");
    let s = s.trim_end_matches('\\').to_string();
    if s.len() == 2 && s.ends_with(':') {
        format!("{}\\", s.to_ascii_uppercase())
    } else {
        s
    }
}

/// Scan `opts.root`. The tree is shared through an `RwLock` so a UI can read partial results while
/// `tree_out` is being filled; pass `None` to let the scan create it.
pub fn scan(opts: &ScanOptions, progress: &Progress, tree_out: Option<Arc<RwLock<Tree>>>) -> std::io::Result<ScanResult> {
    let start = Instant::now();
    let root_name = root_display_name(&opts.root);
    let tree = tree_out.unwrap_or_else(|| Arc::new(RwLock::new(Tree::new(&root_name))));
    {
        let mut t = tree.write().unwrap();
        if t.len() > 1 {
            *t = Tree::new(&root_name);
        }
    }

    let mut fallback_reason = None;
    if matches!(opts.method, Method::Auto | Method::Mft) {
        match mft::scan_volume(&opts.root, progress, &tree) {
            Ok(()) => {
                return Ok(ScanResult {
                    tree,
                    method: Method::Mft,
                    elapsed: start.elapsed(),
                    inaccessible: Vec::new(),
                    cancelled: progress.is_cancelled(),
                    fallback_reason: None,
                });
            }
            Err(e) => {
                if opts.method == Method::Mft {
                    return Err(e);
                }
                fallback_reason = Some(e.to_string());
                // Reset anything a partial MFT attempt may have written.
                *tree.write().unwrap() = Tree::new(&root_name);
                progress.files.store(0, Ordering::Relaxed);
                progress.dirs.store(0, Ordering::Relaxed);
                progress.bytes.store(0, Ordering::Relaxed);
            }
        }
    }

    let strategy = if opts.method == Method::FindFirst { enumerate::Strategy::FindFirst } else { enumerate::Strategy::Auto };
    let threads = if opts.threads > 0 {
        opts.threads
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 32) * 2
    };
    let inaccessible = walk::parallel_walk(&opts.root, strategy, threads, progress, &tree);
    Ok(ScanResult {
        tree,
        method: if strategy == enumerate::Strategy::FindFirst { Method::FindFirst } else { Method::Walk },
        elapsed: start.elapsed(),
        inaccessible,
        cancelled: progress.is_cancelled(),
        fallback_reason,
    })
}
