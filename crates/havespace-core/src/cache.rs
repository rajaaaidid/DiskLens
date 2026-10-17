//! Scan cache: the last scan of each drive/folder, so reopening it is instant.
//!
//! One file per scanned root in `%LOCALAPPDATA%\HaveSpace\cache` (override with `HAVESPACE_CACHE_DIR`):
//! `DLCACHE1`, a little JSON header ([`CacheMeta`], readable without touching the tree), then the tree
//! snapshot compressed with LZ4. Writes go to a temp file that is renamed into place, so a crash never
//! leaves a half-written cache. Only the most recent [`KEEP`] caches are kept.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::scan::{Inaccessible, Method};
use crate::tree::Tree;

const MAGIC: &[u8; 8] = b"DLCACHE1";
/// How many cached scans to keep (oldest are removed).
pub const KEEP: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CacheMeta {
    /// Scanned root as displayed, e.g. `C:\` or `D:\Games`.
    pub root: String,
    /// When the scan finished (Unix ms).
    pub scanned_at_ms: u64,
    pub method: Method,
    pub fallback_reason: Option<String>,
    pub elapsed_ms: u64,
    pub inaccessible: Vec<Inaccessible>,
    pub size: u64,
    pub files: u64,
    pub dirs: u64,
}

pub fn cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("HAVESPACE_CACHE_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    static MIGRATE: std::sync::Once = std::sync::Once::new();
    MIGRATE.call_once(|| {
        let _ = migrate_from_disklens(&base);
    });
    base.join("HaveSpace").join("cache")
}

/// HaveSpace was called DiskLens up to 0.2.0. Move `<base>\DiskLens\cache` to `<base>\HaveSpace\cache`
/// so earlier scans still show in *Recent scans* and the Changes tab, unless the new folder already
/// exists. Removes the old `DiskLens` folder only if nothing else is left in it. Returns whether it moved.
pub fn migrate_from_disklens(base: &Path) -> io::Result<bool> {
    let (old_dir, new_dir) = (base.join("DiskLens"), base.join("HaveSpace"));
    let (old, new) = (old_dir.join("cache"), new_dir.join("cache"));
    if !old.is_dir() || new.exists() {
        return Ok(false);
    }
    fs::create_dir_all(&new_dir)?;
    fs::rename(&old, &new)?;
    let _ = fs::remove_dir(&old_dir);
    Ok(true)
}

/// File name for a root: readable prefix plus a hash of the case-folded path.
pub fn file_name_for(root: &str) -> String {
    let lower = root.trim_end_matches('\\').to_lowercase();
    let readable: String = lower.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).take(40).collect();
    // FNV-1a, stable across runs and platforms.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in lower.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{readable}-{h:016x}.dlc")
}

pub fn path_for(root: &str) -> PathBuf {
    cache_dir().join(file_name_for(root))
}

/// Save a finished scan. Returns the file written.
pub fn save(meta: &CacheMeta, tree: &Tree) -> io::Result<PathBuf> {
    let dir = cache_dir();
    fs::create_dir_all(&dir)?;
    let header = serde_json::to_vec(meta).map_err(io::Error::other)?;
    let body = lz4_flex::compress_prepend_size(&tree.to_bytes());
    let path = dir.join(file_name_for(&meta.root));
    let tmp = path.with_extension("tmp");
    {
        let mut f = io::BufWriter::new(fs::File::create(&tmp)?);
        f.write_all(MAGIC)?;
        f.write_all(&(header.len() as u32).to_le_bytes())?;
        f.write_all(&header)?;
        f.write_all(&body)?;
        f.flush()?;
    }
    fs::rename(&tmp, &path)?;
    prune(KEEP);
    Ok(path)
}

fn read_header(f: &mut impl Read) -> io::Result<CacheMeta> {
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a HaveSpace cache file"));
    }
    let mut len = [0u8; 4];
    f.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 64 << 20 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "cache header too large"));
    }
    let mut header = vec![0u8; len];
    f.read_exact(&mut header)?;
    serde_json::from_slice(&header).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Load the cached scan of `root`.
pub fn load(root: &str) -> io::Result<(CacheMeta, Tree)> {
    load_file(&path_for(root))
}

pub fn load_file(path: &Path) -> io::Result<(CacheMeta, Tree)> {
    let mut f = io::BufReader::new(fs::File::open(path)?);
    let meta = read_header(&mut f)?;
    let mut body = Vec::new();
    f.read_to_end(&mut body)?;
    let raw = lz4_flex::decompress_size_prepended(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tree = Tree::from_bytes(&raw)?;
    Ok((meta, tree))
}

/// Headers of all cached scans, newest first (reads only the small JSON header of each file).
pub fn list() -> Vec<CacheMeta> {
    let Ok(rd) = fs::read_dir(cache_dir()) else { return Vec::new() };
    let mut v: Vec<CacheMeta> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "dlc"))
        .filter_map(|e| fs::File::open(e.path()).ok().and_then(|mut f| read_header(&mut f).ok()))
        .collect();
    v.sort_by(|a, b| b.scanned_at_ms.cmp(&a.scanned_at_ms));
    v
}

/// Remove the cache of one root (e.g. when the user rescans and the old one is stale).
pub fn remove(root: &str) -> io::Result<()> {
    match fs::remove_file(path_for(root)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Keep only the newest `keep` cache files (these are HaveSpace's own files in its own folder).
fn prune(keep: usize) {
    let Ok(rd) = fs::read_dir(cache_dir()) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "dlc"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in files.into_iter().skip(keep) {
        let _ = fs::remove_file(p);
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
