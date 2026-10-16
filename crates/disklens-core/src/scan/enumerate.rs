//! Enumerate one directory into `RawEntry`s without per-file stat calls.
//!
//! Strategies, best first:
//! 1. `GetFileInformationByHandleEx(FileIdExtdDirectoryInfo)`: batched like FindFirstFileEx, but every
//!    entry also carries allocation size, a 128-bit file id (hard-link dedup) and the reparse tag.
//! 2. `FileIdBothDirectoryInfo`: same with a 64-bit id; the reparse tag is in `EaSize`.
//! 3. `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)`: works everywhere (FAT, many
//!    network shares). No file id, so hard links can't be detected; size on disk is estimated by
//!    rounding up to the cluster size.

use std::io;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::tree::{flags, RawEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Start at IdExtd and downgrade automatically when a volume doesn't support it.
    Auto,
    FindFirst,
}

/// Shared, monotonically downgrading choice of API for one scan (0 = IdExtd, 1 = IdBoth, 2 = FindFirst).
pub struct Level(AtomicU8);

impl Level {
    pub fn new(s: Strategy) -> Self {
        Level(AtomicU8::new(if s == Strategy::FindFirst { 2 } else { 0 }))
    }
    fn get(&self) -> u8 {
        self.0.load(Ordering::Relaxed)
    }
    fn downgrade_to(&self, v: u8) {
        self.0.fetch_max(v, Ordering::Relaxed);
    }
}

/// Reparse tags with the "name surrogate" bit point at another file system object
/// (junctions, symlinks, WSL symlinks...). Those are never followed.
pub fn is_name_surrogate(tag: u32) -> bool {
    tag & 0x2000_0000 != 0
}

pub(crate) fn entry_flags(attrs: u32, tag: u32) -> u8 {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        0
    } else if is_name_surrogate(tag) {
        flags::LINK
    } else {
        flags::REPARSE_OTHER
    }
}

#[cfg(windows)]
pub use win::{cluster_size, enumerate_dir, to_wide_long};

#[cfg(windows)]
mod win {
    use super::*;
    use std::mem::offset_of;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Storage::FileSystem::*;

    /// `\\?\`-prefixed, NUL-terminated wide path (no MAX_PATH limit).
    pub fn to_wide_long(path: &str) -> Vec<u16> {
        let mut s = String::with_capacity(path.len() + 8);
        if !path.starts_with(r"\\?\") {
            if let Some(unc) = path.strip_prefix(r"\\") {
                s.push_str(r"\\?\UNC\");
                s.push_str(unc);
            } else {
                s.push_str(r"\\?\");
                s.push_str(path);
            }
        } else {
            s.push_str(path);
        }
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn cluster_size(root: &str) -> u64 {
        let w: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
        let ok = unsafe { GetDiskFreeSpaceW(w.as_ptr(), &mut spc, &mut bps, &mut free, &mut total) };
        if ok != 0 && spc > 0 && bps > 0 {
            spc as u64 * bps as u64
        } else {
            4096
        }
    }

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn name_from(ptr: *const u16, bytes: u32) -> String {
        let s = unsafe { std::slice::from_raw_parts(ptr, bytes as usize / 2) };
        String::from_utf16_lossy(s)
    }

    fn is_dot(name: &str) -> bool {
        name == "." || name == ".."
    }

    fn open_dir_by_path(path: &str) -> io::Result<Handle> {
        let wide = to_wide_long(path);
        let h = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Handle(h))
        }
    }
    /// Enumerate `path` (a plain path like `C:\Users`) into `out`.
    /// `buf` is a reusable scratch buffer (u64 for 8-byte alignment).
    pub fn enumerate_dir(path: &str, level: &Level, cluster: u64, buf: &mut Vec<u64>, out: &mut Vec<RawEntry>) -> io::Result<()> {
        if level.get() >= 2 {
            return find_first(path, cluster, out);
        }
        let h = open_dir_by_path(path)?;
        if buf.len() < 32 * 1024 {
            buf.resize(32 * 1024, 0); // 256 KiB
        }
        let bytes = (buf.len() * 8) as u32;
        let mut class = if level.get() == 0 { FileIdExtdDirectoryInfo } else { FileIdBothDirectoryInfo };
        let mut first = true;
        loop {
            let ok = unsafe { GetFileInformationByHandleEx(h.0, class, buf.as_mut_ptr().cast(), bytes) };
            if ok == 0 {
                let err = unsafe { GetLastError() };
                if err == ERROR_NO_MORE_FILES {
                    return Ok(());
                }
                if first && matches!(err, ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED | ERROR_INVALID_FUNCTION | ERROR_INVALID_LEVEL) {
                    // This volume doesn't support the class: downgrade for the rest of the scan.
                    if class == FileIdExtdDirectoryInfo {
                        level.downgrade_to(1);
                        class = FileIdBothDirectoryInfo;
                        continue;
                    }
                    level.downgrade_to(2);
                    drop(h);
                    return find_first(path, cluster, out);
                }
                return Err(io::Error::from_raw_os_error(err as i32));
            }
            first = false;
            let base = buf.as_ptr() as *const u8;
            let mut off = 0usize;
            loop {
                let p = unsafe { base.add(off) };
                let next;
                if class == FileIdExtdDirectoryInfo {
                    let e = unsafe { std::ptr::read_unaligned(p as *const FILE_ID_EXTD_DIR_INFO) };
                    next = e.NextEntryOffset;
                    let name = name_from(unsafe { p.add(offset_of!(FILE_ID_EXTD_DIR_INFO, FileName)) } as *const u16, e.FileNameLength);
                    if !is_dot(&name) {
                        let is_dir = e.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                        out.push(RawEntry {
                            name,
                            is_dir,
                            size: if is_dir { 0 } else { e.EndOfFile as u64 },
                            alloc: if is_dir { 0 } else { e.AllocationSize as u64 },
                            mtime: e.LastWriteTime as u64,
                            flags: entry_flags(e.FileAttributes, e.ReparsePointTag),
                            file_id: Some(u128::from_le_bytes(e.FileId.Identifier)),
                        });
                    }
                } else {
                    let e = unsafe { std::ptr::read_unaligned(p as *const FILE_ID_BOTH_DIR_INFO) };
                    next = e.NextEntryOffset;
                    let name = name_from(unsafe { p.add(offset_of!(FILE_ID_BOTH_DIR_INFO, FileName)) } as *const u16, e.FileNameLength);
                    if !is_dot(&name) {
                        let is_dir = e.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                        out.push(RawEntry {
                            name,
                            is_dir,
                            size: if is_dir { 0 } else { e.EndOfFile as u64 },
                            alloc: if is_dir { 0 } else { e.AllocationSize as u64 },
                            mtime: e.LastWriteTime as u64,
                            // For reparse points EaSize holds the reparse tag.
                            flags: entry_flags(e.FileAttributes, e.EaSize),
                            file_id: if e.FileId == 0 { None } else { Some(e.FileId as u64 as u128) },
                        });
                    }
                }
                if next == 0 {
                    break;
                }
                off += next as usize;
            }
        }
    }

    fn find_first(path: &str, cluster: u64, out: &mut Vec<RawEntry>) -> io::Result<()> {
        let pattern = if path.ends_with('\\') { format!("{path}*") } else { format!("{path}\\*") };
        let wide = to_wide_long(&pattern);
        let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
        let h = unsafe {
            FindFirstFileExW(
                wide.as_ptr(),
                FindExInfoBasic,
                (&mut data as *mut WIN32_FIND_DATAW).cast(),
                FindExSearchNameMatch,
                null(),
                FIND_FIRST_EX_LARGE_FETCH,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            let err = unsafe { GetLastError() };
            if err == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        loop {
            let len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
            let name = String::from_utf16_lossy(&data.cFileName[..len]);
            if !is_dot(&name) {
                let is_dir = data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                let size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
                let mtime = ((data.ftLastWriteTime.dwHighDateTime as u64) << 32) | data.ftLastWriteTime.dwLowDateTime as u64;
                out.push(RawEntry {
                    name,
                    is_dir,
                    size: if is_dir { 0 } else { size },
                    alloc: if is_dir { 0 } else { size.div_ceil(cluster) * cluster },
                    mtime,
                    flags: entry_flags(data.dwFileAttributes, data.dwReserved0),
                    file_id: None,
                });
            }
            if unsafe { FindNextFileW(h, &mut data) } == 0 {
                break;
            }
        }
        unsafe { FindClose(h) };
        Ok(())
    }
}

/// Portable fallback so the engine and its tests build on non-Windows hosts.
#[cfg(not(windows))]
pub fn enumerate_dir(path: &str, _level: &Level, _cluster: u64, _buf: &mut Vec<u64>, out: &mut Vec<RawEntry>) -> io::Result<()> {

    use std::os::unix::fs::MetadataExt;
    for e in std::fs::read_dir(path)? {
        let e = e?;
        let md = e.metadata()?;
        let ft = md.file_type();
        let is_dir = ft.is_dir();
        out.push(RawEntry {
            name: e.file_name().to_string_lossy().into_owned(),
            is_dir,
            size: if is_dir { 0 } else { md.len() },
            alloc: if is_dir { 0 } else { md.blocks() * 512 },
            mtime: 0,
            flags: if ft.is_symlink() { flags::LINK } else { 0 },
            file_id: if is_dir { None } else { Some(((md.dev() as u128) << 64) | md.ino() as u128) },
        });
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn cluster_size(_root: &str) -> u64 {
    4096
}
