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
