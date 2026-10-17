use disklens_core::hogs::{find_hogs, Severity};
use disklens_core::tree::{RawEntry, Tree, ROOT};

fn file(name: &str, size: u64) -> RawEntry {
    RawEntry { name: name.into(), is_dir: false, size, alloc: size, mtime: 0, flags: 0, file_id: None }
}
fn dir(name: &str) -> RawEntry {
    RawEntry { name: name.into(), is_dir: true, size: 0, alloc: 0, mtime: 0, flags: 0, file_id: None }
}

/// C:\ with the classic suspects.
fn sample() -> Tree {
    let mut t = Tree::new("C:\\");
    let r = t.add_children(ROOT, &[file("hiberfil.sys", 6000), file("pagefile.sys", 9000), dir("Windows"), dir("Users"), dir("Windows.old")]);
    let windows = r + 2;
    let users = r + 3;
    let old = r + 4;
    let w = t.add_children(windows, &[dir("WinSxS"), dir("Temp"), dir("SoftwareDistribution")]);
    t.add_children(w, &[file("a.dll", 3000)]);
    let tmp = t.add_children(w + 1, &[file("x.tmp", 200), dir("Temp")]);
    t.add_children(tmp + 1, &[file("nested.tmp", 50)]); // nested Temp: counted once, inside the outer one
    let sd = t.add_children(w + 2, &[dir("Download")]);
    t.add_children(sd, &[file("kb.cab", 700)]);
    t.add_children(old, &[file("old.wim", 4000)]);
    let me = t.add_children(users, &[dir("me")]);
    let m = t.add_children(me, &[dir("AppData"), dir("proj")]);
    let ad = t.add_children(m, &[dir("Local")]);
    let local = t.add_children(ad, &[dir("Google"), dir("Docker")]);
    let g = t.add_children(local, &[dir("Chrome")]);
    let ch = t.add_children(g, &[dir("User Data")]);
    let ud = t.add_children(ch, &[dir("Default")]);
    let def = t.add_children(ud, &[dir("Cache")]);
    t.add_children(def, &[file("f_000001", 800)]);
    t.add_children(local + 1, &[file("docker_data.vhdx", 5000)]);
    let p = t.add_children(m + 1, &[dir("node_modules"), dir(".git")]);
    let nm = t.add_children(p, &[file("index.js", 100), dir("dep")]);
    let dep = t.add_children(nm + 1, &[dir("node_modules")]); // nested: not counted separately
    t.add_children(dep, &[file("x.js", 40)]);
    t.add_children(p + 1, &[file("pack", 300)]);
    t
}

#[test]
fn finds_every_kind_with_correct_sizes() {
    let t = sample();
    let hogs = find_hogs(&t);
    let get = |id: &str| hogs.iter().find(|h| h.id == id).unwrap_or_else(|| panic!("missing {id}"));
    assert_eq!(get("pagefile").size, 9000);
    assert_eq!(get("hiberfil").size, 6000);
    assert_eq!(get("hiberfil").severity, Severity::System);
    assert_eq!(get("winsxs").size, 3000);
    assert_eq!(get("windowsold").size, 4000);
    assert_eq!(get("wupdate").size, 700);
    assert_eq!(get("vhdx").size, 5000);
    assert_eq!(get("browsercache").size, 800);
    assert_eq!(get(".git".trim_start_matches('.')).size, 300);
    // Nested matches count once.
    assert_eq!(get("temp").size, 250);
    assert_eq!(get("temp").count, 1);
    assert_eq!(get("node_modules").size, 140);
    assert_eq!(get("node_modules").count, 1);
    assert!(get("temp").recyclable);
    assert!(!get("winsxs").recyclable);
    // Largest first.
    assert!(hogs.windows(2).all(|w| w[0].size >= w[1].size));
    assert_eq!(get("browsercache").items[0].path, r"C:\Users\me\AppData\Local\Google\Chrome\User Data\Default\Cache");
}

#[test]
fn deleted_items_drop_out() {
    let mut t = sample();
    let hogs = find_hogs(&t);
    let vhdx = hogs.iter().find(|h| h.id == "vhdx").unwrap().items[0].node;
    t.mark_deleted(vhdx);
    assert!(find_hogs(&t).iter().all(|h| h.id != "vhdx"));
}

#[test]
fn cache_named_folders_outside_browsers_are_not_browser_caches() {
    let mut t = Tree::new("C:\\");
    let a = t.add_children(ROOT, &[dir("MyApp")]);
    let c = t.add_children(a, &[dir("Cache")]);
    t.add_children(c, &[file("x", 10)]);
    assert!(find_hogs(&t).iter().all(|h| h.id != "browsercache"));
}
