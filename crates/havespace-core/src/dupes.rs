//! Duplicate files: identical contents under different names.
//!
//! Done in two steps so the tree is only locked briefly: [`candidates`] picks files from the tree
//! (live, counted, at least `min_size`, sharing their size with another file), then [`find`] reads them
//! from disk without touching the tree:
//!
//! 1. **Sample:** hash the first and last 64 KiB of every candidate. Files no bigger than that are
//!    hashed whole here and are done.
//! 2. **Full:** files that still match their size and sample are hashed whole (BLAKE3).
//! 3. **Identity:** names that turn out to be the same file (a hard link the scan couldn't detect, e.g.
//!    on the `FindFirstFileExW` path) are merged, so they never count as a duplicate.
//!
//! Extra names of hard-linked files (`HARDLINK_DUP`) and links are never candidates. Cloud placeholders
//! (OneDrive files that are online-only) are skipped and never opened, so finding duplicates can't start
//! a download; files are also opened with `FILE_FLAG_OPEN_NO_RECALL` as a second guard.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use serde::Serialize;

use crate::analysis::is_counted_file;
use crate::tree::{flags, NodeId, Tree, NONE};

/// Bytes hashed from each end of a file in the sample step.
const SAMPLE: u64 = 64 * 1024;
const READ_BUF: usize = 1 << 20;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub node: NodeId,
    pub path: String,
    pub size: u64,
    /// Last write time as Windows FILETIME.
    pub mtime: u64,
}

pub struct Candidates {
    pub files: Vec<Candidate>,
    /// Online-only cloud files that were left out.
    pub skipped_cloud: u64,
}

/// Files under `root` that could have a duplicate: live, counted, at least `min_size` bytes (and never
/// empty), and not alone in their size.
pub fn candidates(t: &Tree, root: NodeId, min_size: u64) -> Candidates {
    let live = t.live_mask();
    // Nodes inside `root`: parents always have smaller ids, so one forward pass marks the subtree.
    let mut inside = vec![false; t.len()];
    inside[root as usize] = true;
    let mut by_size: HashMap<u64, Vec<NodeId>> = HashMap::new();
    let mut skipped_cloud = 0;
    for i in root as usize + 1..t.len() {
        let p = t.parent[i];
        if p == NONE || !inside[p as usize] {
            continue;
        }
        inside[i] = true;
        let id = i as NodeId;
        if !is_counted_file(t, &live, id) || t.size[i] < min_size.max(1) {
            continue;
        }
        if t.has_flag(id, flags::REPARSE_OTHER) {
            skipped_cloud += 1;
            continue;
        }
        by_size.entry(t.size[i]).or_default().push(id);
    }
    let files = by_size
        .into_values()
        .filter(|v| v.len() > 1)
        .flatten()
        .map(|id| Candidate { node: id, path: t.path(id), size: t.size[id as usize], mtime: t.mtime[id as usize] })
        .collect();
    Candidates { files, skipped_cloud }
}

/// Live progress, shared with the UI; `cancel` stops the search between reads.
#[derive(Default)]
pub struct DupeProgress {
    /// 1 = sampling, 2 = comparing whole files.
    pub phase: AtomicU8,
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    cancel: AtomicBool,
}

impl DupeProgress {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn start_phase(&self, phase: u8, files: u64, bytes: u64) {
        self.files_done.store(0, Ordering::Relaxed);
        self.bytes_done.store(0, Ordering::Relaxed);
        self.files_total.store(files, Ordering::Relaxed);
        self.bytes_total.store(bytes, Ordering::Relaxed);
        self.phase.store(phase, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeFile {
    pub node: NodeId,
    pub path: String,
    /// Unix ms, 0 if unknown.
    pub mtime_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeSet {
    /// Size of one copy.
    pub size: u64,
    /// Sorted by path.
    pub files: Vec<DupeFile>,
}

impl DupeSet {
    /// Bytes that removing all copies but one would free.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64).saturating_sub(1)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupeReport {
    /// Largest waste first.
    pub sets: Vec<DupeSet>,
    /// Sum of `wasted()` over all sets.
    pub wasted: u64,
    /// Files in all sets, beyond the first copy of each.
    pub extra_copies: u64,
    /// Files that shared their size with another one, and so were looked at.
    pub candidates: u64,
    pub bytes_read: u64,
    pub skipped_cloud: u64,
    /// Files that couldn't be opened or read (in use, access denied, gone since the scan).
    pub unreadable: u64,
    pub cancelled: bool,
    pub elapsed_ms: u64,
}

pub fn filetime_to_ms(ft: u64) -> i64 {
    if ft == 0 {
        0
    } else {
        (ft / 10_000) as i64 - 11_644_473_600_000
    }
}

#[cfg(windows)]
fn open(path: &str) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
    const FILE_FLAG_OPEN_NO_RECALL: u32 = 0x0010_0000;
    std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_ALL).custom_flags(FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_OPEN_NO_RECALL).open(path)
}

#[cfg(not(windows))]
fn open(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Volume serial and file index: the same for every name of one file. `None` for online-only files,
/// which must not be read.
#[cfg(windows)]
fn identity(f: &std::fs::File) -> std::io::Result<Option<(u32, u64)>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
    const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if info.dwFileAttributes & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0 {
        return Ok(None);
    }
    Ok(Some((info.dwVolumeSerialNumber, (info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64)))
}

#[cfg(not(windows))]
fn identity(_f: &std::fs::File) -> std::io::Result<Option<(u32, u64)>> {
    Ok(Some((0, 0)))
}

/// Read up to `n` bytes into the hasher, counting progress. Errors on cancel.
fn feed(f: &mut std::fs::File, hasher: &mut blake3::Hasher, mut n: u64, buf: &mut [u8], p: &DupeProgress) -> std::io::Result<()> {
    while n > 0 {
        if p.is_cancelled() {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let want = (buf.len() as u64).min(n) as usize;
        let got = f.read(&mut buf[..want])?;
        if got == 0 {
            // The file shrank since the scan: it no longer matches its size group.
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        hasher.update(&buf[..got]);
        n -= got as u64;
        p.bytes_done.fetch_add(got as u64, Ordering::Relaxed);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Hashed {
    hash: [u8; 32],
    id: (u32, u64),
}

/// Sample (or, for small files, whole-file) hash of one file. `Ok(None)`: skip it (online-only).
fn sample_hash(c: &Candidate, buf: &mut [u8], p: &DupeProgress) -> std::io::Result<Option<Hashed>> {
    let mut f = open(&c.path)?;
    let Some(id) = identity(&f)? else { return Ok(None) };
    let mut h = blake3::Hasher::new();
    h.update(&c.size.to_le_bytes());
    if c.size <= 2 * SAMPLE {
        feed(&mut f, &mut h, c.size, buf, p)?;
    } else {
        feed(&mut f, &mut h, SAMPLE, buf, p)?;
        f.seek(SeekFrom::Start(c.size - SAMPLE))?;
        feed(&mut f, &mut h, SAMPLE, buf, p)?;
    }
    Ok(Some(Hashed { hash: *h.finalize().as_bytes(), id }))
}

fn full_hash(c: &Candidate, buf: &mut [u8], p: &DupeProgress) -> std::io::Result<Option<Hashed>> {
    let mut f = open(&c.path)?;
    let Some(id) = identity(&f)? else { return Ok(None) };
    let mut h = blake3::Hasher::new();
    feed(&mut f, &mut h, c.size, buf, p)?;
    Ok(Some(Hashed { hash: *h.finalize().as_bytes(), id }))
}

enum Outcome {
    Hashed(Hashed),
    /// Online-only cloud file.
    Skipped,
    Unreadable,
    /// Not read because the search was cancelled.
    NotRead,
}

/// Run `work` over `items` on `threads` workers; results in input order.
fn parallel<T: Sync, F: Fn(&T, &mut [u8]) -> Outcome + Sync>(items: &[T], threads: usize, p: &DupeProgress, work: F) -> Vec<Outcome> {
    let next = AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Outcome>>> = (0..items.len()).map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                let mut buf = vec![0u8; READ_BUF];
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= items.len() || p.is_cancelled() {
                        break;
                    }
                    let r = work(&items[i], &mut buf);
                    p.files_done.fetch_add(1, Ordering::Relaxed);
                    *slots[i].lock().unwrap() = Some(r);
                }
            });
        }
    });
    slots.into_iter().map(|m| m.into_inner().unwrap().unwrap_or(Outcome::NotRead)).collect()
}

fn outcome(r: std::io::Result<Option<Hashed>>) -> Outcome {
    match r {
        Ok(Some(h)) => Outcome::Hashed(h),
        Ok(None) => Outcome::Skipped,
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Outcome::NotRead,
        Err(_) => Outcome::Unreadable,
    }
}

/// Group `files` (with their hashes) by (size, hash), keeping groups of two or more.
type Group = Vec<(Candidate, Hashed)>;

fn regroup(files: Group) -> Vec<Group> {
    let mut m: HashMap<(u64, [u8; 32]), Group> = HashMap::new();
    for (c, h) in files {
        m.entry((c.size, h.hash)).or_default().push((c, h));
    }
    m.into_values().filter(|g| g.len() > 1).collect()
}

/// Read the candidates and return the sets of identical files. `threads` = parallel readers (0 = 4).
pub fn find(c: Candidates, threads: usize, p: &DupeProgress) -> DupeReport {
    let start = std::time::Instant::now();
    let threads = if threads == 0 { 4 } else { threads };
    let mut report = DupeReport { candidates: c.files.len() as u64, skipped_cloud: c.skipped_cloud, ..Default::default() };

    // 1. Sample.
    let sample_bytes = c.files.iter().map(|f| f.size.min(2 * SAMPLE)).sum();
    p.start_phase(1, c.files.len() as u64, sample_bytes);
    let res = parallel(&c.files, threads, p, |f, buf| outcome(sample_hash(f, buf, p)));
    report.bytes_read += p.bytes_done.load(Ordering::Relaxed);
    let mut small = Vec::new();
    let mut big = Vec::new();
    for (f, r) in c.files.into_iter().zip(res) {
        match r {
            Outcome::Hashed(h) if f.size <= 2 * SAMPLE => small.push((f, h)),
            Outcome::Hashed(h) => big.push((f, h)),
            Outcome::Skipped => report.skipped_cloud += 1,
            Outcome::Unreadable => report.unreadable += 1,
            Outcome::NotRead => {}
        }
    }
    let mut done = regroup(small);
    let to_read: Vec<Candidate> = regroup(big).into_iter().flatten().map(|(c, _)| c).collect();

    // 2. Whole files.
    if !p.is_cancelled() {
        p.start_phase(2, to_read.len() as u64, to_read.iter().map(|f| f.size).sum());
        let res = parallel(&to_read, threads, p, |f, buf| outcome(full_hash(f, buf, p)));
        report.bytes_read += p.bytes_done.load(Ordering::Relaxed);
        let mut hashed = Vec::new();
        for (f, r) in to_read.into_iter().zip(res) {
            match r {
                Outcome::Hashed(h) => hashed.push((f, h)),
                Outcome::Skipped => report.skipped_cloud += 1,
                Outcome::Unreadable => report.unreadable += 1,
                Outcome::NotRead => {}
            }
        }
        done.extend(regroup(hashed));
    }
    report.cancelled = p.is_cancelled();
    // A cancelled whole-file step leaves only the small files' sets, which are complete and exact.

    // 3. One entry per actual file (merge names of one hard-linked file).
    for g in done {
        let mut seen = Vec::new();
        let mut files: Vec<DupeFile> = Vec::new();
        let size = g[0].0.size;
        for (c, h) in g {
            if seen.contains(&h.id) {
                continue;
            }
            seen.push(h.id);
            files.push(DupeFile { node: c.node, path: c.path, mtime_ms: filetime_to_ms(c.mtime) });
        }
        if files.len() > 1 {
            files.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
            report.sets.push(DupeSet { size, files });
        }
    }
    report.sets.sort_by(|a, b| b.wasted().cmp(&a.wasted()).then(a.files[0].path.cmp(&b.files[0].path)));
    report.wasted = report.sets.iter().map(DupeSet::wasted).sum();
    report.extra_copies = report.sets.iter().map(|s| s.files.len() as u64 - 1).sum();
    report.elapsed_ms = start.elapsed().as_millis() as u64;
    report
}

/// The report with items since sent to the Recycle Bin removed (sets left with one file disappear).
pub fn without_deleted(r: &DupeReport, t: &Tree) -> DupeReport {
    let live = t.live_mask();
    let mut out = DupeReport { sets: Vec::new(), ..r.clone() };
    for s in &r.sets {
        let files: Vec<DupeFile> = s.files.iter().filter(|f| live.get(f.node as usize).copied().unwrap_or(false)).cloned().collect();
        if files.len() > 1 {
            out.sets.push(DupeSet { size: s.size, files });
        }
    }
    out.sets.sort_by(|a, b| b.wasted().cmp(&a.wasted()));
    out.wasted = out.sets.iter().map(DupeSet::wasted).sum();
    out.extra_copies = out.sets.iter().map(|s| s.files.len() as u64 - 1).sum();
    out
}
