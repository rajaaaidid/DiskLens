use havespace_core::cache::{self, CacheMeta};
use havespace_core::export::{export, iso8601, Format, Options};
use havespace_core::scan::{Inaccessible, Method};
use havespace_core::tree::{flags, RawEntry, Tree, ROOT};

fn file(name: &str, size: u64, mtime: u64) -> RawEntry {
    RawEntry { name: name.into(), is_dir: false, size, alloc: size.div_ceil(4096) * 4096, mtime, flags: 0, file_id: None }
}
fn dir(name: &str) -> RawEntry {
    RawEntry { name: name.into(), is_dir: true, size: 0, alloc: 0, mtime: 0, flags: 0, file_id: None }
}

/// C:\ { big.iso, docs\ { "a, b.txt", résumé.pdf, deep\ { x.mp4 } }, hl.bin (hard-link dup) }
fn sample() -> Tree {
    let mut t = Tree::new("C:\\");
    let mut dup = file("hl.bin", 999, 0);
    dup.flags = flags::HARDLINK_DUP;
    let f = t.add_children(ROOT, &[file("big.iso", 5000, 133_000_000_000_000_000), dir("docs"), dup]);
    let d = t.add_children(f + 1, &[file("a, b.txt", 10, 0), file("résumé.pdf", 300, 0), dir("deep")]);
    t.add_children(d + 2, &[file("x.mp4", 700, 0)]);
    t
}

#[test]
fn snapshot_round_trips_exactly() {
    let mut t = sample();
    t.mark_deleted(t.child_by_name(ROOT, "big.iso").unwrap());
    let back = Tree::from_bytes(&t.to_bytes()).unwrap();
    assert_eq!(back.len(), t.len());
    assert_eq!(back.size, t.size);
    assert_eq!(back.flags, t.flags);
    for i in 0..t.len() as u32 {
        assert_eq!(back.path(i), t.path(i));
        assert_eq!(back.ext_name(i), t.ext_name(i));
    }
    // Damaged data is an error, never a panic.
    let bytes = t.to_bytes();
    for cut in [0, 7, 12, bytes.len() / 2, bytes.len() - 1] {
        assert!(Tree::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    let mut corrupt = bytes.clone();
    corrupt[16] = 0x7F; // parent of node 1 now points past itself
    assert!(Tree::from_bytes(&corrupt).is_err());
}

#[test]
fn cache_saves_lists_and_loads() {
    let dir = std::env::temp_dir().join(format!("havespace-cache-test-{}", std::process::id()));
    std::env::set_var("HAVESPACE_CACHE_DIR", &dir);
    let t = sample();
    let meta = CacheMeta {
        root: "C:\\".into(),
        scanned_at_ms: 1_700_000_000_000,
        method: Method::Mft,
        fallback_reason: None,
        elapsed_ms: 4500,
        inaccessible: vec![Inaccessible { path: "C:\\x".into(), error: "denied".into() }],
        size: t.size[0],
        files: t.files[0] as u64,
        dirs: t.dirs[0] as u64,
    };
    let path = cache::save(&meta, &t).unwrap();
    assert!(path.starts_with(&dir));
    let listed = cache::list();
    assert_eq!(listed, vec![meta.clone()]);
    let (m, back) = cache::load("c:").unwrap(); // case and trailing slash don't matter
    assert_eq!(m, meta);
    assert_eq!(back.size, t.size);
    cache::remove("C:\\").unwrap();
    assert!(cache::list().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn disklens_cache_moves_to_havespace_once() {
    let base = std::env::temp_dir().join(format!("havespace-migrate-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let old = base.join("DiskLens").join("cache");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("c_-0123.dlc"), b"scan").unwrap();

    assert!(cache::migrate_from_disklens(&base).unwrap());
    assert_eq!(std::fs::read(base.join("HaveSpace").join("cache").join("c_-0123.dlc")).unwrap(), b"scan");
    assert!(!base.join("DiskLens").exists(), "empty old folder is removed");

    // Already migrated, or a new install with nothing to move: leave everything alone.
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("other.dlc"), b"x").unwrap();
    assert!(!cache::migrate_from_disklens(&base).unwrap());
    assert!(old.join("other.dlc").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn csv_is_depth_first_largest_first_and_escaped() {
    let t = sample();
    let mut out = Vec::new();
    let s = export(&t, Options { format: Format::Csv, root: ROOT, folders_only: false, max_depth: None }, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "path,type,size,size_on_disk,files,folders,modified,extension,category,flags");
    assert_eq!(s.items, 8);
    let paths: Vec<&str> = lines[1..].iter().map(|l| l.split(',').next().unwrap()).collect();
    assert_eq!(paths[0], "C:\\");
    assert_eq!(paths[1], "C:\\big.iso");
    assert_eq!(paths[2], "C:\\docs");
    assert_eq!(paths[3], "C:\\docs\\deep");
    assert!(text.contains("\"C:\\docs\\a, b.txt\",file,10,"), "commas are quoted");
    assert!(text.contains("C:\\docs\\résumé.pdf,file,300,"));
    assert!(text.contains("C:\\hl.bin,file,0,0,0,0,,bin,System,hardlink"), "hard-link duplicate flagged, 0 bytes, 0 files:\n{text}");
    assert!(text.contains("2022-06-18T04:26:40Z"), "modified time as ISO-8601");

    let mut folders = Vec::new();
    export(&t, Options { format: Format::Csv, root: ROOT, folders_only: true, max_depth: Some(1) }, &mut folders).unwrap();
    let f = String::from_utf8(folders).unwrap();
    assert_eq!(f.lines().count(), 3, "header, root, docs (deep is beyond depth 1):\n{f}");
}

#[test]
fn json_is_valid_nested_and_truncates_at_depth() {
    let mut t = sample();
    t.mark_deleted(t.child_by_name(ROOT, "big.iso").unwrap());
    let mut out = Vec::new();
    export(&t, Options { format: Format::Json, root: ROOT, folders_only: false, max_depth: Some(1) }, &mut out).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
    assert_eq!(v["root"], "C:\\");
    let tree = &v["tree"];
    assert_eq!(tree["size"], 1010);
    let kids = tree["children"].as_array().unwrap();
    assert!(kids.iter().all(|k| k["name"] != "big.iso"), "deleted items are left out");
    let docs = kids.iter().find(|k| k["name"] == "docs").unwrap();
    assert_eq!(docs["files"], 3);
    assert_eq!(docs["truncated"], true);
}

#[test]
fn iso_dates() {
    assert_eq!(iso8601(0), "");
    assert_eq!(iso8601(116_444_736_000_000_000), "1970-01-01T00:00:00Z");
    assert_eq!(iso8601(133_000_000_000_000_000), "2022-06-18T04:26:40Z"); // 1 655 526 400 s after the epoch
}
