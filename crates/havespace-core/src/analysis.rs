//! Post-scan summaries: largest files and folders, per-extension totals.
//!
//! Everything here skips nodes that were sent to the Recycle Bin (and everything inside them), so the
//! summaries stay correct after deletes without a rescan.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use serde::Serialize;

use crate::tree::{flags, NodeId, Tree};

#[derive(Debug, Clone, Serialize)]
pub struct Ranked {
    pub node: NodeId,
    pub path: String,
    pub size: u64,
    pub alloc: u64,
    pub files: u32,
}

/// A live, counted file (not a link or an extra hard-link name).
pub(crate) fn is_counted_file(t: &Tree, live: &[bool], id: NodeId) -> bool {
    live[id as usize] && !t.is_dir(id) && !t.has_flag(id, flags::HARDLINK_DUP | flags::LINK)
}

fn top_by<F: Fn(NodeId) -> Option<u64>>(t: &Tree, k: usize, key: F) -> Vec<Ranked> {
    let mut heap: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::with_capacity(k + 1);
    for id in 0..t.len() as NodeId {
        let Some(s) = key(id) else { continue };
        if heap.len() < k {
            heap.push(Reverse((s, id)));
        } else if s > heap.peek().unwrap().0 .0 {
            heap.pop();
            heap.push(Reverse((s, id)));
        }
    }
    let mut v: Vec<_> = heap.into_iter().map(|Reverse(p)| p).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter()
        .map(|(s, id)| Ranked { node: id, path: t.path(id), size: s, alloc: t.alloc[id as usize], files: t.files[id as usize] })
        .collect()
}

/// The `k` largest counted files (bounded heap, O(n log k)).
pub fn largest_files(t: &Tree, k: usize) -> Vec<Ranked> {
    let live = t.live_mask();
    top_by(t, k, |id| is_counted_file(t, &live, id).then(|| t.size[id as usize]))
}

/// The `k` folders holding the most bytes in their *own* files (not counting subfolders), which
/// surfaces where the data actually sits rather than just `C:\Users`. `size` is that own total.
pub fn largest_folders(t: &Tree, k: usize) -> Vec<Ranked> {
    let live = t.live_mask();
    let mut own = vec![0u64; t.len()];
    for id in 1..t.len() {
        if is_counted_file(t, &live, id as NodeId) {
            own[t.parent[id] as usize] += t.size[id];
        }
    }
    top_by(t, k, |id| (live[id as usize] && t.is_dir(id) && own[id as usize] > 0).then(|| own[id as usize]))
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtTotal {
    pub ext: String,
    pub size: u64,
    pub files: u64,
}

pub fn extension_totals(t: &Tree) -> Vec<ExtTotal> {
    let live = t.live_mask();
    let mut acc = vec![(0u64, 0u64); t.ext_names().len()];
    for id in 0..t.len() as NodeId {
        if !is_counted_file(t, &live, id) {
            continue;
        }
        let e = &mut acc[t.ext[id as usize] as usize];
        e.0 += t.size[id as usize];
        e.1 += 1;
    }
    let mut v: Vec<ExtTotal> = acc
        .into_iter()
        .enumerate()
        .filter(|(_, (_, n))| *n > 0)
        .map(|(i, (size, files))| ExtTotal { ext: if i == 0 { "(none)".into() } else { t.ext_names()[i].clone() }, size, files })
        .collect();
    v.sort_by(|a, b| b.size.cmp(&a.size));
    v
}

// ---------------------------------------------------------------------------------------------
// Age: when data was last modified.

/// 100 ns FILETIME ticks per day.
const FT_DAY: u64 = 864_000_000_000;
/// FILETIME of 1970-01-01.
const FT_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// Current time as a Windows FILETIME.
pub fn now_filetime() -> u64 {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    FT_UNIX_EPOCH + unix.as_nanos() as u64 / 100
}

/// Days since 1970 for a FILETIME, at least 1 (0 means unknown).
pub fn filetime_to_days(ft: u64) -> u32 {
    if ft <= FT_UNIX_EPOCH {
        0
    } else {
        (((ft - FT_UNIX_EPOCH) / FT_DAY) as u32).max(1)
    }
}

/// For every node, the newest modification time of a live counted file inside it (a file's own time
/// for files). 0 where nothing is known. Folders' own timestamps are ignored: they change whenever an
/// entry is added or removed, which says little about the data.
pub fn newest_mtimes(t: &Tree) -> Vec<u64> {
    let live = t.live_mask();
    let mut newest = vec![0u64; t.len()];
    // Children have larger ids than parents, so a reverse sweep is a post-order.
    for i in (0..t.len()).rev() {
        let id = i as NodeId;
        if is_counted_file(t, &live, id) {
            newest[i] = t.mtime[i];
        }
        if i > 0 && live[i] {
            let p = t.parent[i] as usize;
            newest[p] = newest[p].max(newest[i]);
        }
    }
    newest
}

/// Age bands, newest first; the last one (no upper bound) is "5 years or more".
pub const AGE_BANDS_DAYS: [u32; 5] = [30, 182, 365, 730, 1826];
/// Files untouched for at least this long are "stale".
pub const STALE_DAYS: u32 = 730;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgeBand {
    /// Lower bound in days (inclusive); the band ends at the next band's start.
    pub min_days: u32,
    pub size: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgeSummary {
    /// Six bands: < 1 month, 1–6 months, 6–12 months, 1–2 years, 2–5 years, 5+ years.
    pub bands: Vec<AgeBand>,
    /// Files with no usable timestamp.
    pub unknown_size: u64,
    pub unknown_files: u64,
    pub stale_days: u32,
    pub stale_size: u64,
    pub stale_files: u64,
    /// Largest stale files.
    pub stale_largest: Vec<Ranked>,
    /// Folders holding the most stale data in their own files (not counting subfolders).
    pub stale_folders: Vec<Ranked>,
}

fn band_of(age_days: u32) -> usize {
    AGE_BANDS_DAYS.iter().position(|&b| age_days < b).unwrap_or(AGE_BANDS_DAYS.len())
}

/// How the scanned bytes split by last-modified age, plus where the stale data is. `now` is a FILETIME.
pub fn age_summary(t: &Tree, now: u64, k: usize) -> AgeSummary {
    let live = t.live_mask();
    let today = filetime_to_days(now);
    let mut bands: Vec<AgeBand> = std::iter::once(0).chain(AGE_BANDS_DAYS).map(|d| AgeBand { min_days: d, size: 0, files: 0 }).collect();
    let (mut unknown_size, mut unknown_files, mut stale_size, mut stale_files) = (0, 0, 0, 0);
    let mut own_stale = vec![0u64; t.len()];
    let mut is_stale = vec![false; t.len()];
    for i in 1..t.len() {
        let id = i as NodeId;
        if !is_counted_file(t, &live, id) {
            continue;
        }
        let d = filetime_to_days(t.mtime[i]);
        if d == 0 {
            unknown_size += t.size[i];
            unknown_files += 1;
            continue;
        }
        let age = today.saturating_sub(d);
        let b = &mut bands[band_of(age)];
        b.size += t.size[i];
        b.files += 1;
        if age >= STALE_DAYS {
            stale_size += t.size[i];
            stale_files += 1;
            is_stale[i] = true;
            own_stale[t.parent[i] as usize] += t.size[i];
        }
    }
    AgeSummary {
        bands,
        unknown_size,
        unknown_files,
        stale_days: STALE_DAYS,
        stale_size,
        stale_files,
        stale_largest: top_by(t, k, |id| is_stale[id as usize].then(|| t.size[id as usize])),
        stale_folders: top_by(t, k, |id| (live[id as usize] && t.is_dir(id) && own_stale[id as usize] > 0).then(|| own_stale[id as usize])),
    }
}
