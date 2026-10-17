//! Duplicate finder on real fixture files: identical contents, same size but different contents,
//! hard links, size limits, deletes and cancel.

use havespace_core::dupes::{candidates, find, without_deleted, DupeProgress};
use havespace_core::scan::{scan, Method, Progress, ScanOptions};
use havespace_core::tree::{NodeId, Tree, ROOT};
use havespace_fixtures::Fixture;

fn run(root: &std::path::Path, method: Method) -> Tree {
    let progress = Progress::default();
    let opts = ScanOptions { root: root.to_path_buf(), method, threads: 4 };
    let res = scan(&opts, &progress, None).expect("scan");
    let tree = std::mem::take(&mut *res.tree.write().unwrap());
    tree
}


fn find_node(t: &Tree, rel: &str) -> NodeId {
    let mut cur = ROOT;
    for part in rel.split('\\') {
        cur = t.child_by_name(cur, part).unwrap_or_else(|| panic!("{rel}: missing {part}"));
    }
    cur
}

/// Deterministic bytes; `seed` changes every byte.
fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u32).wrapping_mul(2654435761).to_le_bytes()[1] ^ seed).collect()
}

fn names(r: &havespace_core::dupes::DupeReport) -> Vec<Vec<String>> {
    r.sets.iter().map(|s| s.files.iter().map(|f| f.path.rsplit('\\').next().unwrap().to_string()).collect()).collect()
}

#[test]
fn finds_identical_files_and_nothing_else() {
    let fx = Fixture::new("dupes").unwrap();
    // A big set: three copies, bigger than the sample so the whole-file step runs.
    let big = bytes(400_000, 1);
    fx.write("a\\movie.mkv", &big).unwrap();
    fx.write("b\\movie copy.mkv", &big).unwrap();
    fx.write("c\\deep\\movie (1).mkv", &big).unwrap();
    // Same size, same first and last 64 KiB, different middle: must not match.
    let mut tricky = big.clone();
    tricky[200_000] ^= 0xff;
    fx.write("d\\almost.mkv", &tricky).unwrap();
    // A small pair (hashed whole in the sample step).
    let small = bytes(5000, 7);
    fx.write("notes.txt", &small).unwrap();
    fx.write("b\\notes-backup.txt", &small).unwrap();
    // Same size as the small pair, different contents.
    fx.write("other.txt", &bytes(5000, 9)).unwrap();
    // Hard link of the small file: one file, two names; never a duplicate of itself.
    fx.hard_link("other.txt", "other-link.txt").unwrap();
    // Empty files are never duplicates.
    fx.write("empty1", b"").unwrap();
    fx.write("empty2", b"").unwrap();

    let t = run(&fx.root, Method::Walk);
    let r = find(candidates(&t, ROOT, 1), 2, &DupeProgress::default());
    assert!(!r.cancelled);
    assert_eq!(r.unreadable, 0);
    assert_eq!(names(&r), vec![vec!["movie.mkv", "movie copy.mkv", "movie (1).mkv"], vec!["notes-backup.txt", "notes.txt"]]);
    assert_eq!(r.sets[0].size, 400_000);
    assert_eq!(r.wasted, 2 * 400_000 + 5000);
    assert_eq!(r.extra_copies, 3);

    // Minimum size leaves out the small pair.
    let r = find(candidates(&t, ROOT, 10_000), 2, &DupeProgress::default());
    assert_eq!(r.sets.len(), 1);

    // Scoped to a folder: only copies inside it count.
    let r = find(candidates(&t, find_node(&t, "b"), 1), 2, &DupeProgress::default());
    assert!(r.sets.is_empty());
}

#[test]
fn hard_links_are_not_duplicates_even_when_the_scan_missed_them() {
    let fx = Fixture::new("dupes-hl").unwrap();
    fx.write("x.bin", &bytes(300_000, 3)).unwrap();
    fx.hard_link("x.bin", "sub\\y.bin").unwrap();
    // FindFirstFileExW can't see file ids, so both names are counted in the tree. The finder must still
    // see they are one file.
    let t = run(&fx.root, Method::FindFirst);
    let c = candidates(&t, ROOT, 1);
    assert_eq!(c.files.len(), 2, "both names should reach the finder");
    let r = find(c, 2, &DupeProgress::default());
    assert!(r.sets.is_empty(), "{:?}", names(&r));
}

#[test]
fn deleted_items_drop_out() {
    let fx = Fixture::new("dupes-del").unwrap();
    let data = bytes(200_000, 5);
    fx.write("one.iso", &data).unwrap();
    fx.write("two.iso", &data).unwrap();
    fx.write("sub\\three.iso", &data).unwrap();
    let mut t = run(&fx.root, Method::Walk);
    let r = find(candidates(&t, ROOT, 1), 2, &DupeProgress::default());
    assert_eq!(r.sets[0].files.len(), 3);
    t.mark_deleted(find_node(&t, "sub"));
    let r2 = without_deleted(&r, &t);
    assert_eq!(r2.sets[0].files.len(), 2);
    assert_eq!(r2.wasted, 200_000);
    t.mark_deleted(find_node(&t, "two.iso"));
    assert!(without_deleted(&r, &t).sets.is_empty());
}

#[test]
fn cancel_stops_without_results() {
    let fx = Fixture::new("dupes-cancel").unwrap();
    let data = bytes(300_000, 2);
    fx.write("a.bin", &data).unwrap();
    fx.write("b.bin", &data).unwrap();
    let t = run(&fx.root, Method::Walk);
    let p = DupeProgress::default();
    p.cancel();
    let r = find(candidates(&t, ROOT, 1), 2, &p);
    assert!(r.cancelled);
    assert!(r.sets.is_empty());
    assert_eq!(r.unreadable, 0);
}
