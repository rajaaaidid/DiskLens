//! Age bands, stale data and newest-modification roll-up on a hand-built tree.

use havespace_core::analysis::{age_summary, filetime_to_days, newest_mtimes, STALE_DAYS};
use havespace_core::tree::{flags, RawEntry, Tree};

const DAY: u64 = 864_000_000_000;
/// 2026-10-18 as a FILETIME.
const NOW: u64 = 116_444_736_000_000_000 + 20_744 * DAY;

fn file(name: &str, size: u64, age_days: u64) -> RawEntry {
    RawEntry { name: name.into(), is_dir: false, size, alloc: size, mtime: if age_days == u64::MAX { 0 } else { NOW - age_days * DAY }, flags: 0, file_id: None }
}

fn dir(name: &str) -> RawEntry {
    // A folder's own time is recent; it must not make old data look new.
    RawEntry { name: name.into(), is_dir: true, size: 0, alloc: 0, mtime: NOW, flags: 0, file_id: None }
}

#[test]
fn bands_stale_and_newest() {
    let mut t = Tree::new("C:\\");
    let first = t.add_children(0, &[dir("old"), dir("mixed"), file("fresh.txt", 100, 3), file("nodate.bin", 50, u64::MAX)]);
    let (old, mixed) = (first, first + 1);
    t.add_children(old, &[file("a.iso", 4000, 3000), file("b.zip", 1000, 800)]);
    let mut link = file("link", 999, 1);
    link.flags = flags::HARDLINK_DUP;
    t.add_children(mixed, &[file("c.doc", 200, 40), file("d.doc", 300, 400), link]);

    let s = age_summary(&t, NOW, 10);
    let sizes: Vec<u64> = s.bands.iter().map(|b| b.size).collect();
    // < 1 month, 1–6 months, 6–12 months, 1–2 years, 2–5 years, 5+ years
    assert_eq!(sizes, vec![100, 200, 0, 300, 1000, 4000]);
    assert_eq!((s.unknown_size, s.unknown_files), (50, 1));
    assert_eq!(s.stale_days, STALE_DAYS);
    assert_eq!((s.stale_size, s.stale_files), (5000, 2));
    assert_eq!(s.stale_largest.iter().map(|r| r.size).collect::<Vec<_>>(), vec![4000, 1000]);
    assert_eq!(s.stale_folders.len(), 1);
    assert_eq!(s.stale_folders[0].path, r"C:\old");
    assert_eq!(s.stale_folders[0].size, 5000);

    let n = newest_mtimes(&t);
    assert_eq!(filetime_to_days(n[old as usize]), filetime_to_days(NOW - 800 * DAY));
    // The extra hard-link name (1 day old) doesn't count.
    assert_eq!(filetime_to_days(n[mixed as usize]), filetime_to_days(NOW - 40 * DAY));
    assert_eq!(filetime_to_days(n[0]), filetime_to_days(NOW - 3 * DAY));

    // Deleting the newest file makes its folder older.
    t.mark_deleted(first + 2);
    assert_eq!(filetime_to_days(newest_mtimes(&t)[0]), filetime_to_days(NOW - 40 * DAY));
    assert_eq!(age_summary(&t, NOW, 10).bands[0].size, 0);
}
