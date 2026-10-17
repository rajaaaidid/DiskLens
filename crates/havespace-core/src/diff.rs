//! Compare two scans of the same root: what grew, shrank, appeared or disappeared since the last scan.
//!
//! Both trees are walked together from the root, matching children by name (case-insensitive, as on
//! Windows). Only logical sizes are compared. Two lists come out:
//!
//! - **items:** files that changed size, plus files and whole folders that are new or gone (a new folder
//!   is one entry, not one per file inside it);
//! - **folders:** folders present in both scans, ranked by the change *directly inside them* (their own
//!   files, plus new or gone subfolders), so `C:\Users` doesn't hide the `Downloads` that really grew.
//!
//! Folders that couldn't be read in either scan, and links, are skipped: their size isn't known, so any
//! difference would be noise. Files whose hard-link status changed are skipped too, because which name of
//! a hard-linked file carries the bytes can differ from scan to scan.

use std::collections::HashMap;

use serde::Serialize;

use crate::tree::{flags, NodeId, Tree, ROOT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Removed,
    Grew,
    Shrank,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
    pub is_dir: bool,
    /// Size in the earlier scan (0 if added). For folders, the whole folder.
    pub before: u64,
    /// Size now (0 if removed). For folders, the whole folder.
    pub after: u64,
    /// Bytes gained (negative: freed). For folders, only the change directly inside them.
    pub delta: i64,
    /// Node in the newer tree; `None` for removed items.
    pub node: Option<NodeId>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanDiff {
    /// Root total in the earlier scan and now.
    pub before: u64,
    pub after: u64,
    pub files_before: u64,
    pub files_after: u64,
    /// Sum of all growth and of all shrinkage across every changed item (not just the listed ones).
    pub grew: u64,
    pub shrank: u64,
    pub added: u64,
    pub removed: u64,
    pub changed: u64,
    pub items: Vec<Change>,
    pub folders: Vec<Change>,
}

#[derive(Clone, Copy)]
enum Side {
    Old,
    New,
}

/// One change before paths are resolved (only the top `k` ever need a path).
struct Raw {
    side: Side,
    id: NodeId,
    kind: ChangeKind,
    before: u64,
    after: u64,
    delta: i64,
}

fn skip_dir(t: &Tree, id: NodeId) -> bool {
    t.has_flag(id, flags::INACCESSIBLE | flags::LINK)
}

/// Compare `old` (the earlier scan) with `new`, keeping the `k` largest item and folder changes.
pub fn diff(old: &Tree, new: &Tree, k: usize) -> ScanDiff {
    let (old_live, new_live) = (old.live_mask(), new.live_mask());
    let mut items: Vec<Raw> = Vec::new();
    let mut folders: Vec<Raw> = Vec::new();
    let (mut grew, mut shrank) = (0u64, 0u64);
    let (mut added, mut removed, mut changed) = (0u64, 0u64, 0u64);

    let mut by_name: HashMap<String, NodeId> = HashMap::new();
    let mut stack = vec![(ROOT, ROOT)];
    while let Some((o, n)) = stack.pop() {
        by_name.clear();
        by_name.extend(old.children(o).filter(|&c| old_live[c as usize]).map(|c| (old.name(c).to_lowercase(), c)));
        let mut direct = 0i64;
        let mut item = |r: Raw, items: &mut Vec<Raw>| {
            direct += r.delta;
            if r.delta > 0 {
                grew += r.delta as u64;
            } else {
                shrank += r.delta.unsigned_abs();
            }
            match r.kind {
                ChangeKind::Added => added += 1,
                ChangeKind::Removed => removed += 1,
                _ => changed += 1,
            }
            items.push(r);
        };
        for c in new.children(n).filter(|&c| new_live[c as usize]) {
            let after = new.size[c as usize];
            let added_item = Raw { side: Side::New, id: c, kind: ChangeKind::Added, before: 0, after, delta: after as i64 };
            let Some(oc) = by_name.remove(&new.name(c).to_lowercase()) else {
                if after > 0 {
                    item(added_item, &mut items);
                }
                continue;
            };
            let before = old.size[oc as usize];
            match (old.is_dir(oc), new.is_dir(c)) {
                (true, true) => {
                    if !skip_dir(old, oc) && !skip_dir(new, c) {
                        stack.push((oc, c));
                    }
                }
                (false, false) => {
                    let dup = |t: &Tree, id| t.has_flag(id, flags::HARDLINK_DUP);
                    if before != after && dup(old, oc) == dup(new, c) {
                        let kind = if after > before { ChangeKind::Grew } else { ChangeKind::Shrank };
                        item(Raw { side: Side::New, id: c, kind, before, after, delta: after as i64 - before as i64 }, &mut items);
                    }
                }
                // A file replaced by a folder of the same name, or the other way round.
                _ => {
                    if before > 0 {
                        item(Raw { side: Side::Old, id: oc, kind: ChangeKind::Removed, before, after: 0, delta: -(before as i64) }, &mut items);
                    }
                    if after > 0 {
                        item(added_item, &mut items);
                    }
                }
            }
        }
        for (_, oc) in by_name.drain() {
            let before = old.size[oc as usize];
            if before > 0 {
                item(Raw { side: Side::Old, id: oc, kind: ChangeKind::Removed, before, after: 0, delta: -(before as i64) }, &mut items);
            }
        }
        if direct != 0 {
            let kind = if direct > 0 { ChangeKind::Grew } else { ChangeKind::Shrank };
            folders.push(Raw { side: Side::New, id: n, kind, before: old.size[o as usize], after: new.size[n as usize], delta: direct });
        }
    }

    let resolve = |mut v: Vec<Raw>| -> Vec<Change> {
        // Largest change first, then (after the cut) ties ordered by path so the output is stable.
        v.sort_unstable_by_key(|r| std::cmp::Reverse(r.delta.unsigned_abs()));
        v.truncate(k);
        let mut out: Vec<Change> = v
            .into_iter()
            .map(|r| {
                let t = match r.side {
                    Side::Old => old,
                    Side::New => new,
                };
                Change {
                    path: t.path(r.id),
                    kind: r.kind,
                    is_dir: t.is_dir(r.id),
                    before: r.before,
                    after: r.after,
                    delta: r.delta,
                    node: matches!(r.side, Side::New).then_some(r.id),
                }
            })
            .collect();
        out.sort_by(|a, b| b.delta.unsigned_abs().cmp(&a.delta.unsigned_abs()).then_with(|| a.path.cmp(&b.path)));
        out
    };

    ScanDiff {
        before: old.size[ROOT as usize],
        after: new.size[ROOT as usize],
        files_before: old.files[ROOT as usize] as u64,
        files_after: new.files[ROOT as usize] as u64,
        grew,
        shrank,
        added,
        removed,
        changed,
        items: resolve(items),
        folders: resolve(folders),
    }
}
