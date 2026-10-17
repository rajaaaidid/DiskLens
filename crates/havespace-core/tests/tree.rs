//! Size aggregation in the arena tree.

use havespace_core::tree::{flags, RawEntry, Tree, ROOT};

fn file(name: &str, size: u64) -> RawEntry {
    RawEntry { name: name.into(), is_dir: false, size, alloc: size.div_ceil(4096) * 4096, mtime: 0, flags: 0, file_id: None }
}

fn dir(name: &str) -> RawEntry {
    RawEntry { name: name.into(), is_dir: true, size: 0, alloc: 0, mtime: 0, flags: 0, file_id: None }
}

/// C:\ { a.txt 100, big.bin 5000, docs\ { x.pdf 300, deep\ { y.mp4 7000 } }, empty\ }
fn sample() -> Tree {
    let mut t = Tree::new("C:\\");
    let first = t.add_children(ROOT, &[file("a.txt", 100), file("big.bin", 5000), dir("docs"), dir("empty")]);
    let docs = first + 2;
    let empty = first + 3;
    let d = t.add_children(docs, &[file("x.pdf", 300), dir("deep")]);
    t.add_children(d + 1, &[file("y.mp4", 7000)]);
    t.add_children(empty, &[]);
    t
}

#[test]
fn folder_totals_are_subtree_sums() {
    let t = sample();
    assert_eq!(t.size[0], 100 + 5000 + 300 + 7000);
    assert_eq!(t.files[0], 4);
    assert_eq!(t.dirs[0], 3);
    let docs = t.child_by_name(ROOT, "docs").unwrap();
    assert_eq!(t.size[docs as usize], 7300);
    assert_eq!(t.files[docs as usize], 2);
    assert_eq!(t.dirs[docs as usize], 1);
    let empty = t.child_by_name(ROOT, "empty").unwrap();
    assert_eq!(t.size[empty as usize], 0);
    assert!(t.has_flag(empty, flags::SCANNED));
}

#[test]
fn incremental_totals_match_full_recompute() {
    let mut t = sample();
    let before = (t.size.clone(), t.alloc.clone(), t.files.clone(), t.dirs.clone());
    t.recompute_totals();
    assert_eq!(before, (t.size.clone(), t.alloc.clone(), t.files.clone(), t.dirs.clone()));
}

#[test]
fn partial_tree_is_consistent_before_children_arrive() {
    // Streaming: totals must be right at every step, not only at the end.
    let mut t = Tree::new("C:\\");
    let f = t.add_children(ROOT, &[file("a", 10), dir("d")]);
    assert_eq!(t.size[0], 10);
    t.add_children(f + 1, &[file("b", 5)]);
    assert_eq!(t.size[0], 15);
    assert_eq!(t.size[(f + 1) as usize], 5);
}

#[test]
fn delete_subtracts_from_every_ancestor_without_rescan() {
    let mut t = sample();
    let docs = t.child_by_name(ROOT, "docs").unwrap();
    let deep = t.child_by_name(docs, "deep").unwrap();
    t.mark_deleted(deep);
    assert_eq!(t.size[docs as usize], 300);
    assert_eq!(t.size[0], 100 + 5000 + 300);
    assert_eq!(t.files[0], 3);
    assert_eq!(t.dirs[0], 2);
    // Deleting twice is a no-op.
    t.mark_deleted(deep);
    assert_eq!(t.size[0], 5400);
    // A full recompute agrees.
    let s = t.size[0];
    t.recompute_totals();
    assert_eq!(t.size[0], s);
}

#[test]
fn links_and_hardlink_duplicates_count_zero() {
    let mut t = Tree::new("C:\\");
    let mut dup = file("second-name.bin", 1000);
    dup.flags = flags::HARDLINK_DUP;
    let mut link = dir("junction");
    link.flags = flags::LINK;
    t.add_children(ROOT, &[file("first-name.bin", 1000), dup, link]);
    assert_eq!(t.size[0], 1000);
    assert_eq!(t.files[0], 1);
}

#[test]
fn paths_names_and_extensions() {
    let t = sample();
    let docs = t.child_by_name(ROOT, "DOCS").unwrap();
    let deep = t.child_by_name(docs, "deep").unwrap();
    let y = t.children(deep).next().unwrap();
    assert_eq!(t.path(y), "C:\\docs\\deep\\y.mp4");
    assert_eq!(t.ext_name(y), "mp4");
    assert_eq!(t.ext_name(docs), "");
}

#[test]
fn memory_per_node_is_compact() {
    let mut t = Tree::new("C:\\");
    let entries: Vec<RawEntry> = (0..100_000).map(|i| file(&format!("file_{i:06}.dat"), i)).collect();
    t.add_children(ROOT, &entries);
    let per_node = t.heap_bytes() as f64 / t.len() as f64;
    // ~53 bytes of columns + ~15 bytes of name; budget is 300 MB per 1M files.
    assert!(per_node < 120.0, "{per_node} bytes/node");
}
