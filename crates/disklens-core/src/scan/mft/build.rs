//! Turn parsed MFT batches into a `Tree`.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use super::record::{Batch, Rec};
use crate::scan::enumerate::is_name_surrogate;
use crate::scan::Progress;
use crate::tree::{flags, NodeId, RawEntry, Tree, ROOT};

/// The root directory of every NTFS volume is file record 5.
pub const ROOT_RECORD: u32 = 5;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const NO_SLOT: u64 = u64::MAX;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildStats {
    /// In-use records with a name whose parent chain doesn't reach the root (skipped).
    pub orphans: u64,
    /// Extra hard-link names (shown in the tree, counted once).
    pub hardlink_names: u64,
}

/// All parsed batches plus an index from record number to its (merged) base record.
pub struct Records {
    batches: Vec<Batch>,
    /// Record number -> (batch << 32 | index) of its base record, or `NO_SLOT`.
    slot: Vec<u64>,
    /// Names that live in extension records: (base record, batch, name index), sorted by base.
    ext_names: Vec<(u32, u32, u32)>,
}

impl Records {
    /// Index `batches` (in any order) and fold extension records into their base records.
    pub fn new(batches: Vec<Batch>, count: usize) -> Self {
        let mut slot = vec![NO_SLOT; count];
        let mut ext = Vec::new();
        for (bi, b) in batches.iter().enumerate() {
            for (ri, r) in b.recs.iter().enumerate() {
                if r.rec as usize >= count {
                    continue;
                }
                if r.base == r.rec {
                    slot[r.rec as usize] = ((bi as u64) << 32) | ri as u64;
                } else {
                    ext.push((bi, ri));
                }
            }
        }
        let mut me = Records { batches, slot, ext_names: Vec::new() };
        for (bi, ri) in ext {
            let e: Rec = me.batches[bi].recs[ri];
            let Some((sb, si)) = me.locate(e.base) else { continue };
            let b = &mut me.batches[sb].recs[si];
            if !b.has_data && e.has_data {
                b.has_data = true;
                b.size = e.size;
                b.alloc = e.alloc;
            }
            if b.reparse_tag == 0 {
                b.reparse_tag = e.reparse_tag;
            }
            for k in 0..e.name_count as u32 {
                me.ext_names.push((e.base, bi as u32, e.first_name + k));
            }
        }
        me.ext_names.sort_unstable_by_key(|x| x.0);
        me
    }

    fn locate(&self, rec: u32) -> Option<(usize, usize)> {
        let s = *self.slot.get(rec as usize)?;
        (s != NO_SLOT).then(|| ((s >> 32) as usize, (s & 0xFFFF_FFFF) as usize))
    }

    pub fn get(&self, rec: u32) -> Option<&Rec> {
        self.locate(rec).map(|(b, i)| &self.batches[b].recs[i])
    }

    pub fn len(&self) -> usize {
        self.slot.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slot.is_empty()
    }

    pub fn heap_bytes(&self) -> usize {
        self.batches.iter().map(Batch::heap_bytes).sum::<usize>() + self.slot.capacity() * 8 + self.ext_names.capacity() * 12
    }

    /// All names of a base record as (batch, name index): its own, then any from extension records.
    fn names_of(&self, rec: u32, ext_from: &mut usize, out: &mut Vec<(u32, u32)>) {
        out.clear();
        if let Some((b, i)) = self.locate(rec) {
            let r = &self.batches[b].recs[i];
            for k in 0..r.name_count as u32 {
                out.push((b as u32, r.first_name + k));
            }
        }
        while *ext_from < self.ext_names.len() && self.ext_names[*ext_from].0 < rec {
            *ext_from += 1;
        }
        while *ext_from < self.ext_names.len() && self.ext_names[*ext_from].0 == rec {
            let (_, b, n) = self.ext_names[*ext_from];
            out.push((b, n));
            *ext_from += 1;
        }
    }

    fn name(&self, batch: u32, idx: u32) -> &str {
        self.batches[batch as usize].name(idx as usize)
    }
}

#[derive(Clone, Copy)]
struct Link {
    child: u32,
    batch: u32,
    name: u32,
    primary: bool,
}

/// Build `tree` (which must be fresh: just its root) breadth-first from the root directory, so every
/// folder exists before its children are added.
pub fn build_tree(tree: &mut Tree, recs: &Records, progress: &Progress) -> BuildStats {
    let n = recs.len();
    let mut stats = BuildStats::default();
    let live_dir = |i: u32, seq: u16| recs.get(i).is_some_and(|p| p.is_dir && (seq == 0 || seq == p.seq));

    // Every valid (parent, child, name) link; a record's first valid name is its counted one.
    let mut links: Vec<(u32, Link)> = Vec::with_capacity(n);
    let mut named = 0u64;
    let mut names = Vec::new();
    let mut ext_cursor = 0usize;
    for rec in 0..n as u32 {
        if recs.get(rec).is_none() {
            continue;
        }
        recs.names_of(rec, &mut ext_cursor, &mut names);
        if rec == ROOT_RECORD || names.is_empty() {
            continue;
        }
        named += 1;
        let mut primary = true;
        for &(b, k) in &names {
            let nr = recs.batches[b as usize].names[k as usize];
            if nr.parent == rec || !live_dir(nr.parent, nr.parent_seq) {
                continue;
            }
            links.push((nr.parent, Link { child: rec, batch: b, name: k, primary }));
            if !primary {
                stats.hardlink_names += 1;
            }
            primary = false;
        }
    }

    // Group links by parent (counting sort).
    let mut start = vec![0u32; n + 1];
    for (p, _) in &links {
        start[*p as usize + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut sorted = vec![Link { child: 0, batch: 0, name: 0, primary: false }; links.len()];
    let mut fill = start.clone();
    for &(p, l) in &links {
        sorted[fill[p as usize] as usize] = l;
        fill[p as usize] += 1;
    }
    drop(links);
    drop(fill);

    let mut visited = vec![false; n];
    let mut reached = 0u64;
    let mut queue: VecDeque<(u32, NodeId)> = VecDeque::new();
    if recs.get(ROOT_RECORD).is_some() {
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
        for l in &sorted[start[rec as usize] as usize..start[rec as usize + 1] as usize] {
            let Some(p) = recs.get(l.child) else { continue };
            let surrogate = p.reparse_tag != 0 && is_name_surrogate(p.reparse_tag);
            let mut fl = if surrogate {
                flags::LINK
            } else if p.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                flags::REPARSE_OTHER
            } else {
                0
            };
            if !l.primary {
                fl |= flags::HARDLINK_DUP;
            }
            let (size, alloc) = if p.is_dir || !p.has_data { (0, 0) } else { (p.size, p.alloc) };
            if p.is_dir {
                dirs += 1;
            } else if l.primary && !surrogate {
                files += 1;
                bytes += size;
            }
            entries.push(RawEntry { name: recs.name(l.batch, l.name).to_string(), is_dir: p.is_dir, size, alloc, mtime: p.mtime, flags: fl, file_id: None });
            kids.push(l.child);
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
    stats.orphans = named.saturating_sub(reached);
    stats
}
