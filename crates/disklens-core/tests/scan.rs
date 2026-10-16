//! End-to-end scans of real fixture trees: hard links, junctions, symlinks, access errors, cancel.

use disklens_core::scan::{scan, Method, Progress, ScanOptions};
use disklens_core::tree::{flags, NodeId, Tree, ROOT};
use disklens_fixtures::{generate, Fixture};

fn run(root: &std::path::Path, method: Method) -> (Tree, disklens_core::ScanResult) {
    let progress = Progress::default();
    let opts = ScanOptions { root: root.to_path_buf(), method, threads: 4 };
    let res = scan(&opts, &progress, None).expect("scan");
    let tree = std::mem::take(&mut *res.tree.write().unwrap());
    (tree, res)
}

fn find(t: &Tree, rel: &str) -> NodeId {
    let mut cur = ROOT;
    for part in rel.split('\\') {
        cur = t.child_by_name(cur, part).unwrap_or_else(|| panic!("{rel}: missing {part}"));
    }
    cur
}

#[test]
fn plain_tree_sizes_and_counts() {
    let fx = Fixture::new("plain").unwrap();
    fx.file("a.txt", 1234).unwrap();
    fx.file("sub\\b.bin", 1 << 20).unwrap();
    fx.file("sub\\deeper\\c.mp4", 3 << 20).unwrap();
    fx.dir("empty").unwrap();
    let (t, res) = run(&fx.root, Method::Walk);
    assert_eq!(res.method, Method::Walk);
    assert_eq!(t.size[0], 1234 + (1 << 20) + (3 << 20));
    assert_eq!(t.files[0], 3);
    assert_eq!(t.dirs[0], 3);
    assert_eq!(t.size[find(&t, "sub") as usize], (1 << 20) + (3 << 20));
    // Size on disk is at least the logical size for non-resident files.
    assert!(t.alloc[find(&t, "sub\\b.bin") as usize] >= 1 << 20);
    assert!(res.inaccessible.is_empty());
}

#[test]
fn hard_links_are_counted_once() {
    let fx = Fixture::new("hardlink").unwrap();
    fx.file("orig\\data.bin", 500_000).unwrap();
    fx.hard_link("orig\\data.bin", "other\\same-data.bin").unwrap();
    fx.hard_link("orig\\data.bin", "other\\third-name.bin").unwrap();
    fx.file("other\\unique.bin", 1000).unwrap();
    let (t, _) = run(&fx.root, Method::Walk);
    assert_eq!(t.size[0], 501_000, "three names of one file must count once");
    assert_eq!(t.files[0], 2);
    let dups = (0..t.len() as NodeId).filter(|&i| t.has_flag(i, flags::HARDLINK_DUP)).count();
    assert_eq!(dups, 2);
    // All three names are still visible in the tree.
    for p in ["orig\\data.bin", "other\\same-data.bin", "other\\third-name.bin"] {
        find(&t, p);
    }
}

#[test]
fn junctions_are_not_followed() {
    let fx = Fixture::new("junction").unwrap();
    fx.file("target\\payload.bin", 2_000_000).unwrap();
    fx.junction("link-to-target", "target").unwrap();
    // A junction pointing back up must not cause a cycle.
    fx.junction("target\\loop", ".").unwrap();
    let (t, _) = run(&fx.root, Method::Walk);
    assert_eq!(t.size[0], 2_000_000);
    assert_eq!(t.files[0], 1);
    let link = find(&t, "link-to-target");
    assert!(t.has_flag(link, flags::LINK));
    assert_eq!(t.children(link).len(), 0);
    assert!(t.has_flag(find(&t, "target\\loop"), flags::LINK));
}

#[test]
fn symlinks_are_not_followed_when_available() {
    let fx = Fixture::new("symlink").unwrap();
    fx.file("real\\big.bin", 3_000_000).unwrap();
    let dir_ok = fx.symlink_dir("dirlink", "real").is_ok();
    let file_ok = fx.symlink_file("filelink.bin", "real\\big.bin").is_ok();
    if !dir_ok && !file_ok {
        eprintln!("skipped: creating symlinks needs Developer Mode or admin");
        return;
    }
    let (t, _) = run(&fx.root, Method::Walk);
    assert_eq!(t.size[0], 3_000_000);
    if dir_ok {
        assert!(t.has_flag(find(&t, "dirlink"), flags::LINK));
    }
    if file_ok {
        assert!(t.has_flag(find(&t, "filelink.bin"), flags::LINK));
    }
}

#[test]
fn generated_tree_matches_generator_totals_with_both_enumerators() {
    let fx = Fixture::new("generated").unwrap();
    let stats = generate(&fx.root, 3000, 6, 4 << 20, 7).unwrap();
    let (walk, _) = run(&fx.root, Method::Walk);
    assert_eq!(walk.files[0] as u64, stats.files);
    assert_eq!(walk.dirs[0] as u64, stats.dirs);
    assert_eq!(walk.size[0], stats.bytes);
    let (ff, res) = run(&fx.root, Method::FindFirst);
    assert_eq!(res.method, Method::FindFirst);
    assert_eq!(ff.size[0], stats.bytes);
    assert_eq!(ff.files[0] as u64, stats.files);
}

#[test]
fn auto_falls_back_to_walk_with_a_reason() {
    let fx = Fixture::new("auto").unwrap();
    fx.file("x.bin", 10).unwrap();
    let (t, res) = run(&fx.root, Method::Auto);
    assert_eq!(t.size[0], 10);
    assert_eq!(res.method, Method::Walk);
    assert!(res.fallback_reason.is_some());
}

#[test]
fn inaccessible_folders_are_reported() {
    let fx = Fixture::new("denied").unwrap();
    fx.file("ok\\a.bin", 100).unwrap();
    fx.file("locked\\secret.bin", 999).unwrap();
    let locked = fx.path("locked");
    // Use the SID: a bare user name can resolve to the machine account when they share a name.
    let who = std::process::Command::new("whoami").args(["/user", "/fo", "csv", "/nh"]).output().ok();
    let sid = who.and_then(|o| String::from_utf8_lossy(&o.stdout).trim().rsplit(',').next().map(|s| s.trim_matches('"').to_string()));
    let Some(sid) = sid.filter(|s| s.starts_with("S-1-")) else {
        eprintln!("skipped: could not determine user SID");
        return;
    };
    let user = format!("*{sid}");
    let deny = std::process::Command::new("icacls").arg(&locked).args(["/deny", &format!("{user}:(RD)")]).output();
    if !deny.map(|o| o.status.success()).unwrap_or(false) {
        eprintln!("skipped: icacls unavailable");
        return;
    }
    let (t, res) = run(&fx.root, Method::Walk);
    // Restore access so the fixture can be cleaned up.
    let _ = std::process::Command::new("icacls").arg(&locked).args(["/remove:d", &user]).output();
    assert_eq!(res.inaccessible.len(), 1, "{:?}", res.inaccessible);
    assert!(res.inaccessible[0].path.ends_with("locked"));
    assert!(t.has_flag(find(&t, "locked"), flags::INACCESSIBLE));
    assert_eq!(t.size[0], 100);
}

#[test]
fn cancel_stops_early_and_keeps_partial_tree() {
    let fx = Fixture::new("cancel").unwrap();
    generate(&fx.root, 2000, 5, 1024, 3).unwrap();
    let progress = Progress::default();
    progress.cancel();
    let res = scan(&ScanOptions { root: fx.root.clone(), method: Method::Walk, threads: 2 }, &progress, None).unwrap();
    assert!(res.cancelled);
    let t = res.tree.read().unwrap();
    // The root batch may or may not have been read, but nothing below it was.
    assert!(t.files[0] < 2000);
}

#[test]
fn live_progress_matches_final_totals() {
    let fx = Fixture::new("progress").unwrap();
    let stats = generate(&fx.root, 1500, 4, 1 << 16, 11).unwrap();
    let progress = Progress::default();
    let res = scan(&ScanOptions { root: fx.root.clone(), method: Method::Walk, threads: 4 }, &progress, None).unwrap();
    let snap = progress.snapshot();
    assert_eq!(snap.files, stats.files);
    assert_eq!(snap.bytes, stats.bytes);
    assert_eq!(res.tree.read().unwrap().size[0], stats.bytes);
}
