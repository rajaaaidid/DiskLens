//! Turn parsed MFT records into a `Tree`.

use std::sync::atomic::Ordering;

use super::record::Parsed;
use crate::scan::enumerate::is_name_surrogate;
use crate::scan::Progress;
use crate::tree::{flags, NodeId, RawEntry, Tree, ROOT};

/// The root directory of every NTFS volume is file record 5.
pub const ROOT_RECORD: u32 = 5;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildStats {
    /// In-use records with a name whose parent chain doesn't reach the root (skipped).
    pub orphans: u64,
    /// Extra hard-link names (shown in the tree, counted once).
    pub hardlink_names: u64,
}

/// Fold extension records into their base records. `records` may arrive in any order.
pub fn merge(records: impl IntoIterator<Item = Parsed>, count: usize) -> Vec<Option<Parsed>> {
    let mut base: Vec<Option<Parsed>> = (0..count).map(|_| None).collect();
    let mut ext = Vec::new();
    for p in records {
        let i = p.rec as usize;
        if i >= count {
            continue;
        }
        if p.base == p.rec {
            base[i] = Some(p);
        } else {
            ext.push(p);
        }
    }
    for e in ext {
        let Some(Some(b)) = base.get_mut(e.base as usize) else { continue };
        b.names.extend(e.names);
        if b.data.is_none() {
            b.data = e.data;
        }
        if b.reparse_tag == 0 {
            b.reparse_tag = e.reparse_tag;
        }
    }
    base
}

/// Build `tree` (which must be fresh: just its root) from merged records, breadth-first from the root
/// directory so every folder exists before its children are added.
pub fn build_tree(tree: &mut Tree, recs: &mut [Option<Parsed>], progress: &Progress) -> BuildStats {
    let n = recs.len();
    let mut stats = BuildStats::default();
    let is_live_dir = |recs: &[Option<Parsed>], i: u32, seq: u16| -> bool {
        match recs.get(i as usize) {
            Some(Some(p)) => p.is_dir && (seq == 0 || seq == p.seq),
            _ => false,
        }
    };

    // (parent record, child record, name index, is_primary) for every valid link.
    let mut links: Vec<(u32, u32, u8, bool)> = Vec::with_capacity(n);
    for (i, slot) in recs.iter().enumerate() {
        let Some(p) = slot else { continue };
        if i as u32 == ROOT_RECORD {
            continue;
        }
        let mut primary_taken = false;
        for (k, (parent, pseq, _)) in p.names.iter().enumerate().take(255) {
            if *parent as usize == i || !is_live_dir(recs, *parent, *pseq) {
                continue;
            }
            links.push((*parent, i as u32, k as u8, !primary_taken));
            if primary_taken {
                stats.hardlink_names += 1;
            }
            primary_taken = true;
        }
    }

    // Group links by parent (counting sort).
    let mut start = vec![0u32; n + 1];
    for l in &links {
        start[l.0 as usize + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut sorted = vec![(0u32, 0u8, false); links.len()];
    let mut fill = start.clone();
    for &(parent, child, k, primary) in &links {
        let slot = &mut fill[parent as usize];
        sorted[*slot as usize] = (child, k, primary);
        *slot += 1;
    }
    drop(links);

    let mut visited = vec![false; n];
    let mut reached = 0u64;
    let mut queue: std::collections::VecDeque<(u32, NodeId)> = std::collections::VecDeque::new();
    if (ROOT_RECORD as usize) < n && recs[ROOT_RECORD as usize].is_some() {
        visited[ROOT_RECORD as usize] = true;
        queue.push_back((ROOT_RECORD, ROOT));
    }
    let mut entries: Vec<RawEntry> = Vec::new();
    let mut kids: Vec<u32> = Vec::new();
    while let Some((rec, node)) = queue.pop_front() {
        if progress.is_cancelled() {
            break;
        }
        entries.clear();
        kids.clear();
        let (mut files, mut dirs, mut bytes) = (0u64, 0u64, 0u64);
        for &(child, k, primary) in &sorted[start[rec as usize] as usize..start[rec as usize + 1] as usize] {
            let Some(p) = recs[child as usize].as_mut() else { continue };
            let name = std::mem::take(&mut p.names[k as usize].2);
            let surrogate = is_name_surrogate(p.reparse_tag) && (p.reparse_tag != 0);
            let mut fl = if surrogate {
                flags::LINK
            } else if p.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                flags::REPARSE_OTHER
            } else {
                0
            };
            if !primary {
                fl |= flags::HARDLINK_DUP;
            }
            let (size, alloc) = if p.is_dir { (0, 0) } else { p.data.unwrap_or((0, 0)) };
            if p.is_dir {
                dirs += 1;
            } else if primary && !surrogate {
                files += 1;
                bytes += size;
            }
            entries.push(RawEntry { name, is_dir: p.is_dir, size, alloc, mtime: p.mtime, flags: fl, file_id: None });
            kids.push(child);
        }
        let first = tree.add_children(node, &entries);
        progress.files.fetch_add(files, Ordering::Relaxed);
        progress.dirs.fetch_add(dirs, Ordering::Relaxed);
        progress.bytes.fetch_add(bytes, Ordering::Relaxed);
        for (k, &child) in kids.iter().enumerate() {
            let e = &entries[k];
            if e.flags & flags::HARDLINK_DUP == 0 {
                reached += 1;
            }
            if e.is_dir && e.flags & flags::LINK == 0 && !visited[child as usize] {
                visited[child as usize] = true;
                queue.push_back((child, first + k as NodeId));
            }
        }
    }
    // Everything named that the walk from the root never reached is orphaned.
    let named = recs.iter().enumerate().filter(|(i, p)| *i as u32 != ROOT_RECORD && p.as_ref().is_some_and(|p| !p.names.is_empty())).count() as u64;
    stats.orphans = named.saturating_sub(reached);
    stats
}
