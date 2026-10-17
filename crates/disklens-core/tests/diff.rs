use disklens_core::diff::{diff, ChangeKind};
use disklens_core::tree::{flags, RawEntry, Tree, ROOT};

fn file(name: &str, size: u64) -> RawEntry {
    RawEntry { name: name.into(), is_dir: false, size, alloc: size, mtime: 0, flags: 0, file_id: None }
}
fn dir(name: &str) -> RawEntry {
    RawEntry { name: name.into(), is_dir: true, size: 0, alloc: 0, mtime: 0, flags: 0, file_id: None }
}
fn with(mut e: RawEntry, f: u8) -> RawEntry {
    e.flags = f;
    e
}

/// C:\ { Users\ { me\ { Downloads\ { a.zip, b.iso }, notes.txt } }, Windows\ { sys.dll }, pagefile.sys }
fn before() -> Tree {
    let mut t = Tree::new("C:\\");
    let r = t.add_children(ROOT, &[dir("Users"), dir("Windows"), file("pagefile.sys", 8000)]);
    let me = t.add_children(r, &[dir("me")]);
    let m = t.add_children(me, &[dir("Downloads"), file("notes.txt", 10)]);
    t.add_children(m, &[file("a.zip", 1000), file("b.iso", 4000)]);
    t.add_children(r + 1, &[file("sys.dll", 500)]);
    t
}

#[test]
fn identical_scans_have_no_changes() {
    let d = diff(&before(), &before(), 10);
    assert!(d.items.is_empty() && d.folders.is_empty());
    assert_eq!((d.grew, d.shrank), (0, 0));
    assert_eq!(d.before, d.after);
}

#[test]
fn finds_grown_new_and_removed_items_and_ranks_folders_by_direct_change() {
    // Downloads: a.zip grew 1000 -> 3000, b.iso removed, new folder "game" (2 files, 6000) appeared.
    // pagefile.sys shrank 8000 -> 5000. Names differ only in case for "users": still the same folder.
    let mut t = Tree::new("C:\\");
    let r = t.add_children(ROOT, &[dir("users"), dir("Windows"), file("pagefile.sys", 5000)]);
    let me = t.add_children(r, &[dir("me")]);
    let m = t.add_children(me, &[dir("Downloads"), file("notes.txt", 10)]);
    let dl = t.add_children(m, &[file("a.zip", 3000), dir("game")]);
    t.add_children(dl + 1, &[file("x.pak", 5000), file("y.pak", 1000)]);
    t.add_children(r + 1, &[file("sys.dll", 500)]);

    let d = diff(&before(), &t, 10);
    assert_eq!(d.before, 13510);
    assert_eq!(d.after, 14510);
    assert_eq!(d.grew, 2000 + 6000);
    assert_eq!(d.shrank, 4000 + 3000);
    assert_eq!((d.added, d.removed, d.changed), (1, 1, 2));

    let summary: Vec<_> = d.items.iter().map(|c| (c.path.as_str(), c.kind, c.delta)).collect();
    assert_eq!(
        summary,
        vec![
            ("C:\\users\\me\\Downloads\\game", ChangeKind::Added, 6000),
            ("C:\\Users\\me\\Downloads\\b.iso", ChangeKind::Removed, -4000),
            ("C:\\pagefile.sys", ChangeKind::Shrank, -3000),
            ("C:\\users\\me\\Downloads\\a.zip", ChangeKind::Grew, 2000),
        ]
    );
    assert!(d.items[0].is_dir && d.items[0].node.is_some());
    assert!(d.items[1].node.is_none(), "removed items have no node in the new tree");

    // Downloads: +6000 +2000 -4000 directly inside; the root: -3000 (pagefile). Users and me: nothing direct.
    let folders: Vec<_> = d.folders.iter().map(|c| (c.path.as_str(), c.kind, c.delta, c.before, c.after)).collect();
    assert_eq!(folders, vec![("C:\\users\\me\\Downloads", ChangeKind::Grew, 4000, 5000, 9000), ("C:\\", ChangeKind::Shrank, -3000, 13510, 14510)]);
}

#[test]
fn keeps_only_the_k_largest_but_totals_cover_everything() {
    let mut a = Tree::new("D:\\");
    let mut b = Tree::new("D:\\");
    let old: Vec<_> = (0..50).map(|i| file(&format!("f{i}"), 100)).collect();
    let new: Vec<_> = (0..50).map(|i| file(&format!("f{i}"), 100 + i)).collect();
    a.add_children(ROOT, &old);
    b.add_children(ROOT, &new);
    let d = diff(&a, &b, 5);
    assert_eq!(d.items.len(), 5);
    assert_eq!(d.items[0].path, "D:\\f49");
    assert_eq!(d.items[4].path, "D:\\f45");
    assert_eq!(d.grew, (1..50).sum::<u64>());
    assert_eq!(d.changed, 49);
}

#[test]
fn skips_unreadable_folders_links_and_hard_link_shuffles() {
    let mut a = Tree::new("C:\\");
    let r = a.add_children(ROOT, &[dir("Secret"), with(dir("Junction"), flags::LINK), dir("WinSxS")]);
    a.add_children(r, &[file("x", 100)]);
    a.add_children(r + 2, &[file("one.dll", 300), with(file("two.dll", 300), flags::HARDLINK_DUP)]);

    // Secret is unreadable now; the counted name of the hard-linked file moved from one.dll to two.dll.
    let mut b = Tree::new("C:\\");
    let r = b.add_children(ROOT, &[with(dir("Secret"), flags::INACCESSIBLE), with(dir("Junction"), flags::LINK), dir("WinSxS")]);
    b.add_children(r + 2, &[with(file("one.dll", 300), flags::HARDLINK_DUP), file("two.dll", 300)]);

    let d = diff(&a, &b, 10);
    assert!(d.items.is_empty(), "{:?}", d.items);
    assert!(d.folders.is_empty(), "{:?}", d.folders);
}

#[test]
fn file_replaced_by_folder_counts_as_removed_and_added() {
    let mut a = Tree::new("C:\\");
    a.add_children(ROOT, &[file("data", 100)]);
    let mut b = Tree::new("C:\\");
    b.add_children(ROOT, &[dir("data")]);
    b.add_children(1, &[file("inner", 250)]);
    let d = diff(&a, &b, 10);
    let kinds: Vec<_> = d.items.iter().map(|c| (c.kind, c.is_dir, c.delta)).collect();
    assert_eq!(kinds, vec![(ChangeKind::Added, true, 250), (ChangeKind::Removed, false, -100)]);
}

#[test]
fn items_recycled_from_the_old_scan_count_as_already_gone() {
    // A scan saved after a Recycle Bin action: the recycled file must not show up as "removed" again.
    let mut a = before();
    let dl = a.child_by_name(ROOT, "Users").and_then(|u| a.child_by_name(u, "me")).and_then(|m| a.child_by_name(m, "Downloads")).unwrap();
    let iso = a.child_by_name(dl, "b.iso").unwrap();
    a.mark_deleted(iso);

    let mut b = Tree::new("C:\\");
    let r = b.add_children(ROOT, &[dir("Users"), dir("Windows"), file("pagefile.sys", 8000)]);
    let me = b.add_children(r, &[dir("me")]);
    let m = b.add_children(me, &[dir("Downloads"), file("notes.txt", 10)]);
    b.add_children(m, &[file("a.zip", 1000)]);
    b.add_children(r + 1, &[file("sys.dll", 500)]);

    let d = diff(&a, &b, 10);
    assert!(d.items.is_empty(), "{:?}", d.items);
    assert_eq!(d.before, d.after);
}
