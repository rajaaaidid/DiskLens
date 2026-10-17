//! Fake directory trees for HaveSpace tests and benchmarks.
//!
//! [`Fixture`] builds a small hand-specified tree (files with exact sizes, hard links, junctions,
//! symlinks) in a temp folder and deletes it on drop. [`generate`] builds a large pseudo-random tree
//! for scan benchmarks.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct Fixture {
    pub root: PathBuf,
    keep: bool,
}

impl Fixture {
    /// Create an empty fixture folder under the system temp dir.
    pub fn new(name: &str) -> io::Result<Self> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("havespace-fx-{}-{}-{}", name, std::process::id(), n));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;
        Ok(Fixture { root, keep: false })
    }

    /// Keep the folder after drop (for debugging).
    pub fn keep(mut self) -> Self {
        self.keep = true;
        self
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn dir(&self, rel: &str) -> io::Result<PathBuf> {
        let p = self.path(rel);
        fs::create_dir_all(&p)?;
        Ok(p)
    }

    /// File of exactly `size` logical bytes (zero-filled, not sparse).
    pub fn file(&self, rel: &str, size: u64) -> io::Result<PathBuf> {
        let p = self.path(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        let f = File::create(&p)?;
        f.set_len(size)?;
        Ok(p)
    }

    pub fn hard_link(&self, existing: &str, new: &str) -> io::Result<PathBuf> {
        let p = self.path(new);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::hard_link(self.path(existing), &p)?;
        Ok(p)
    }

    /// Directory junction (mount point). Needs no special privileges.
    pub fn junction(&self, link: &str, target: &str) -> io::Result<PathBuf> {
        let l = self.path(link);
        let t = self.path(target);
        make_junction(&l, &t)?;
        Ok(l)
    }

    /// Directory symlink. Needs Developer Mode or admin on Windows; returns an error otherwise.
    pub fn symlink_dir(&self, link: &str, target: &str) -> io::Result<PathBuf> {
        let l = self.path(link);
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(self.path(target), &l)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(self.path(target), &l)?;
        Ok(l)
    }

    pub fn symlink_file(&self, link: &str, target: &str) -> io::Result<PathBuf> {
        let l = self.path(link);
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(self.path(target), &l)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(self.path(target), &l)?;
        Ok(l)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.keep {
            // remove_dir_all does not follow junctions/symlinks, so link targets are safe.
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(windows)]
pub fn make_junction(link: &Path, target: &Path) -> io::Result<()> {
    let out = Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

#[cfg(not(windows))]
pub fn make_junction(link: &Path, target: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Totals of a generated tree, for checking scan results.
#[derive(Debug, Default, Clone, Copy)]
pub struct GenStats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
}

/// Deterministic xorshift PRNG.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const EXTS: &[&str] = &["txt", "jpg", "png", "mp4", "zip", "dll", "exe", "json", "js", "log", "pdf", "dat"];

/// Generate roughly `files` files spread over a random folder hierarchy up to `max_depth` deep.
/// File sizes are log-distributed between 0 and ~`max_file` bytes. Files are created with `set_len`
/// (zero-filled), so generation is metadata-bound rather than I/O-bound.
pub fn generate(root: &Path, files: u64, max_depth: u32, max_file: u64, seed: u64) -> io::Result<GenStats> {
    fs::create_dir_all(root)?;
    let mut rng = Rng(seed | 1);
    let mut stats = GenStats::default();
    let mut dirs: Vec<(PathBuf, u32)> = vec![(root.to_path_buf(), 0)];
    let max_bits = 64 - max_file.max(2).leading_zeros() as u64;
    while stats.files < files {
        // Pick a folder, biased towards recent ones so the tree gets depth.
        let i = if dirs.len() > 8 && rng.below(4) != 0 { dirs.len() - 1 - rng.below(8) as usize } else { rng.below(dirs.len() as u64) as usize };
        let (dir, depth) = dirs[i].clone();
        if depth < max_depth && rng.below(6) == 0 {
            let d = dir.join(format!("folder_{}", stats.dirs));
            fs::create_dir(&d)?;
            stats.dirs += 1;
            dirs.push((d, depth + 1));
            continue;
        }
        let n = 1 + rng.below(24);
        for _ in 0..n {
            if stats.files >= files {
                break;
            }
            let bits = rng.below(max_bits);
            let size = if bits == 0 { 0 } else { (1u64 << bits) + rng.below(1u64 << bits) };
            let size = size.min(max_file);
            let ext = EXTS[rng.below(EXTS.len() as u64) as usize];
            let f = File::create(dir.join(format!("file_{}.{}", stats.files, ext)))?;
            if size > 0 {
                f.set_len(size)?;
            }
            stats.files += 1;
            stats.bytes += size;
        }
    }
    Ok(stats)
}
