//! Real Recycle Bin tests. They put a small fixture file into *your* Recycle Bin, so they are ignored by
//! default. Run them explicitly:
//!
//!     cargo test --release -p havespace-core --test recycle -- --ignored --test-threads=1
#![cfg(windows)]

use std::fs::File;
use std::os::windows::io::AsRawHandle;

use havespace_core::actions::{recycle, recycle_bin_capacity, recycle_without_precheck, RecycleItem, TOO_BIG_FOR_BIN};
use havespace_fixtures::Fixture;

#[test]
fn temp_drive_has_a_recycle_bin_capacity() {
    let tmp = std::env::temp_dir();
    let cap = recycle_bin_capacity(&tmp.to_string_lossy());
    assert!(cap.is_some_and(|c| c > 0), "{cap:?}");
}

#[test]
#[ignore = "touches the real Recycle Bin"]
fn small_file_goes_to_the_recycle_bin() {
    let fx = Fixture::new("recycle").unwrap();
    let p = fx.file("HaveSpace-recycle-test-delete-me.txt", 1234).unwrap();
    let out = recycle(vec![RecycleItem { path: p.to_string_lossy().into_owned(), size: 1234 }]).unwrap();
    assert_eq!(out.recycled.len(), 1, "{out:?}");
    assert!(out.failed.is_empty() && out.permanently_deleted.is_empty());
    assert!(!p.exists());
}

fn huge_sparse_file(fx: &Fixture, name: &str) -> (std::path::PathBuf, u64) {
    let p = fx.path(name);
    let size = 8u64 << 40; // 8 TiB logical, ~0 bytes on disk
    let f = File::create(&p).unwrap();
    make_sparse(&f);
    f.set_len(size).unwrap();
    (p, size)
}

/// An item far larger than the Recycle Bin is refused before the shell ever sees it.
#[test]
#[ignore = "touches the real Recycle Bin"]
fn never_deletes_permanently() {
    let fx = Fixture::new("nuke").unwrap();
    let (p, size) = huge_sparse_file(&fx, "HaveSpace-too-big-for-the-bin.bin");
    let out = recycle(vec![RecycleItem { path: p.to_string_lossy().into_owned(), size }]).unwrap();
    assert!(p.exists(), "the file must still exist: {out:?}");
    assert!(out.recycled.is_empty() && out.permanently_deleted.is_empty());
    assert_eq!(out.failed.len(), 1);
    assert_eq!(out.failed[0].1, TOO_BIG_FOR_BIN);
}

/// Documents *why* the pre-check exists: left to itself the shell silently deletes such an item
/// permanently. The tripwire must at least notice and report it. Only a throwaway sparse fixture is lost.
#[test]
#[ignore = "touches the real Recycle Bin"]
fn tripwire_reports_a_permanent_delete_by_the_shell() {
    let fx = Fixture::new("tripwire").unwrap();
    let (p, _) = huge_sparse_file(&fx, "HaveSpace-tripwire-fixture.bin");
    let path = p.to_string_lossy().into_owned();
    let out = recycle_without_precheck(vec![path.clone()]).unwrap();
    assert!(out.recycled.is_empty(), "must not be reported as recycled: {out:?}");
    assert_eq!(out.permanently_deleted.len(), 1, "{out:?}");
    assert!(out.permanently_deleted[0].eq_ignore_ascii_case(&path));
}

fn make_sparse(f: &File) {
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    let mut ret = 0u32;
    let ok = unsafe { DeviceIoControl(f.as_raw_handle() as _, FSCTL_SET_SPARSE, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut ret, std::ptr::null_mut()) };
    assert!(ok != 0, "FSCTL_SET_SPARSE failed: {}", std::io::Error::last_os_error());
}
