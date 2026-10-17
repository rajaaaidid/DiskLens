//! File actions: send to the Recycle Bin and reveal in Explorer.
//!
//! HaveSpace never deletes permanently. The shell's `IFileOperation` with `FOFX_RECYCLEONDELETE` would
//! still *permanently* delete an item it can't recycle (no Recycle Bin on the drive, bin disabled, or item
//! larger than the bin's size limit), and it gives no warning beforehand: `PreDeleteItem` reports the
//! same flags either way (verified: see `tests/recycle.rs`). So:
//!
//! 1. **Pre-check** every item before the shell sees it: it must be on a local volume whose Recycle Bin
//!    is enabled (`NukeOnDelete` = 0) and its size must be under the bin's `MaxCapacity`, both read from
//!    the same per-volume registry settings Explorer uses. Anything else is refused.
//! 2. **Tripwire**: if `PostDeleteItem` ever reports a successful delete that produced no Recycle Bin
//!    item, the operation is aborted immediately and the outcome reports it, rather than continuing.

use std::io;

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleOutcome {
    /// Paths that are now in the Recycle Bin.
    pub recycled: Vec<String>,
    /// Paths that were not touched, with the reason.
    pub failed: Vec<(String, String)>,
    /// Paths the shell deleted without recycling despite the pre-check (should always be empty).
    pub permanently_deleted: Vec<String>,
}

/// One item to recycle: full path and its total size in bytes (logical size of the subtree for folders).
#[derive(Debug, Clone)]
pub struct RecycleItem {
    pub path: String,
    pub size: u64,
}

pub const NO_RECYCLE_BIN: &str = "This drive has no Recycle Bin (or it is turned off), so the item would be deleted permanently. HaveSpace never does that.";
pub const TOO_BIG_FOR_BIN: &str = "Too large for the Recycle Bin on this drive, so Windows would delete it permanently. HaveSpace never does that. Reduce it in smaller pieces or raise the Recycle Bin size.";

/// Fraction of the bin's capacity an item may use (margin for files that grew since the scan).
const CAPACITY_MARGIN: f64 = 0.98;

#[cfg(windows)]
pub use win::{recycle, recycle_bin_capacity, reveal_in_explorer};

#[cfg(windows)]
#[doc(hidden)]
pub use win::recycle_without_precheck;

#[cfg(windows)]
mod win {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use windows::core::{implement, Result as WinResult, HRESULT, HSTRING, PCWSTR};
    use windows::Win32::Foundation::E_ABORT;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem, ILCreateFromPathW, ILFree,
        SHCreateItemFromParsingName, SHOpenFolderAndSelectItems, FILEOPERATION_FLAGS, SIGDN_FILESYSPATH,
    };
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    const FOF_SILENT: u32 = 0x0004;
    const FOF_NOCONFIRMATION: u32 = 0x0010;
    const FOF_ALLOWUNDO: u32 = 0x0040;
    const FOF_NOERRORUI: u32 = 0x0400;
    const FOFX_RECYCLEONDELETE: u32 = 0x0008_0000;
    const FOFX_EARLYFAILURE: u32 = 0x0010_0000;
    const DRIVE_FIXED: u32 = 3;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
        let (k, v) = (wide(subkey), wide(value));
        let mut data = 0u32;
        let mut len = 4u32;
        let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut data as *mut u32).cast(), &mut len) };
        (rc == 0).then_some(data)
    }

    /// Recycle Bin capacity in bytes for the volume holding `path`, or `None` if items on that volume
    /// can't be recycled (not a fixed local drive, no Recycle Bin settings, or bin turned off).
    pub fn recycle_bin_capacity(path: &str) -> Option<u64> {
        let w = wide(path);
        let mut root = [0u16; 512];
        if unsafe { GetVolumePathNameW(w.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return None;
        }
        if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_FIXED {
            return None;
        }
        let mut guid = [0u16; 64];
        if unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), guid.as_mut_ptr(), guid.len() as u32) } == 0 {
            return None;
        }
        let g = String::from_utf16_lossy(&guid[..guid.iter().position(|&c| c == 0).unwrap_or(0)]);
        // "\\?\Volume{...}\" -> "{...}"
        let id = g.trim_start_matches(r"\\?\Volume").trim_end_matches('\\');
        const BB: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket";
        let key = if reg_dword(BB, "UseGlobalSettings") == Some(1) { BB.to_string() } else { format!(r"{BB}\Volume\{id}") };
        if reg_dword(&key, "NukeOnDelete").unwrap_or(0) != 0 {
            return None;
        }
        let mb = reg_dword(&key, "MaxCapacity")?;
        Some(mb as u64 * 1024 * 1024)
    }

    /// Watches the shell's per-item results. A delete that succeeds without producing a Recycle Bin
    /// item was permanent: record it and abort everything that is still queued.
    #[implement(IFileOperationProgressSink)]
    struct Tripwire {
        tripped: Arc<AtomicBool>,
        nuked: Arc<Mutex<Vec<String>>>,
    }

    #[allow(non_snake_case)]
    impl IFileOperationProgressSink_Impl for Tripwire_Impl {
        fn StartOperations(&self) -> WinResult<()> {
            Ok(())
        }
        fn FinishOperations(&self, _hr: HRESULT) -> WinResult<()> {
            Ok(())
        }
        fn PreRenameItem(&self, _f: u32, _i: Option<&IShellItem>, _n: &PCWSTR) -> WinResult<()> {
            Ok(())
        }
        fn PostRenameItem(&self, _f: u32, _i: Option<&IShellItem>, _n: &PCWSTR, _hr: HRESULT, _new: Option<&IShellItem>) -> WinResult<()> {
            Ok(())
        }
        fn PreMoveItem(&self, _f: u32, _i: Option<&IShellItem>, _d: Option<&IShellItem>, _n: &PCWSTR) -> WinResult<()> {
            Ok(())
        }
        fn PostMoveItem(&self, _f: u32, _i: Option<&IShellItem>, _d: Option<&IShellItem>, _n: &PCWSTR, _hr: HRESULT, _new: Option<&IShellItem>) -> WinResult<()> {
            Ok(())
        }
        fn PreCopyItem(&self, _f: u32, _i: Option<&IShellItem>, _d: Option<&IShellItem>, _n: &PCWSTR) -> WinResult<()> {
            Ok(())
        }
        fn PostCopyItem(&self, _f: u32, _i: Option<&IShellItem>, _d: Option<&IShellItem>, _n: &PCWSTR, _hr: HRESULT, _new: Option<&IShellItem>) -> WinResult<()> {
            Ok(())
        }
        fn PreDeleteItem(&self, _f: u32, _i: Option<&IShellItem>) -> WinResult<()> {
            if self.tripped.load(Ordering::SeqCst) {
                return Err(E_ABORT.into());
            }
            Ok(())
        }
        fn PostDeleteItem(&self, _f: u32, item: Option<&IShellItem>, hr: HRESULT, recycled: Option<&IShellItem>) -> WinResult<()> {
            if hr.is_ok() && recycled.is_none() {
                self.tripped.store(true, Ordering::SeqCst);
                let path = item
                    .and_then(|i| unsafe { i.GetDisplayName(SIGDN_FILESYSPATH) }.ok())
                    .map(|p| unsafe {
                        let s = p.to_string().unwrap_or_default();
                        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
                        s
                    })
                    .unwrap_or_default();
                self.nuked.lock().unwrap().push(path);
                return Err(E_ABORT.into());
            }
            Ok(())
        }
        fn PreNewItem(&self, _f: u32, _d: Option<&IShellItem>, _n: &PCWSTR) -> WinResult<()> {
            Ok(())
        }
        fn PostNewItem(&self, _f: u32, _d: Option<&IShellItem>, _n: &PCWSTR, _t: &PCWSTR, _a: u32, _hr: HRESULT, _new: Option<&IShellItem>) -> WinResult<()> {
            Ok(())
        }
        fn UpdateProgress(&self, _total: u32, _done: u32) -> WinResult<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> WinResult<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> WinResult<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> WinResult<()> {
            Ok(())
        }
    }

    /// Run `f` on a fresh STA thread with COM initialised (shell operations require an STA).
    fn on_sta<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::spawn(move || {
            let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            let r = f();
            if init.is_ok() {
                unsafe { CoUninitialize() };
            }
            r
        })
        .join()
        .expect("shell worker thread panicked")
    }

    fn to_io(e: windows::core::Error) -> io::Error {
        io::Error::other(e.message().to_string())
    }

    /// Send items to the Recycle Bin. Items that could only be deleted permanently are refused untouched.
    pub fn recycle(items: Vec<RecycleItem>) -> io::Result<RecycleOutcome> {
        let mut refused = Vec::new();
        let mut ok = Vec::new();
        for it in items {
            match recycle_bin_capacity(&it.path) {
                None => refused.push((it.path, NO_RECYCLE_BIN.to_string())),
                Some(cap) if it.size as f64 > cap as f64 * CAPACITY_MARGIN => refused.push((it.path, TOO_BIG_FOR_BIN.to_string())),
                Some(_) => ok.push(it.path),
            }
        }
        let mut out = if ok.is_empty() { RecycleOutcome::default() } else { recycle_without_precheck(ok)? };
        out.failed.extend(refused);
        Ok(out)
    }

    /// The shell operation alone, with only the tripwire. Exposed for the test that proves the
    /// pre-check is necessary; use [`recycle`].
    pub fn recycle_without_precheck(paths: Vec<String>) -> io::Result<RecycleOutcome> {
        on_sta(move || {
            let tripped = Arc::new(AtomicBool::new(false));
            let nuked = Arc::new(Mutex::new(Vec::new()));
            let op: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }.map_err(to_io)?;
            unsafe {
                op.SetOperationFlags(FILEOPERATION_FLAGS(FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE))
                    .map_err(to_io)?;
                let sink: IFileOperationProgressSink = Tripwire { tripped: tripped.clone(), nuked: nuked.clone() }.into();
                op.Advise(&sink).map_err(to_io)?;
            }
            let mut out = RecycleOutcome::default();
            let mut queued = Vec::new();
            for p in paths {
                match unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(p.as_str()), None) } {
                    Ok(item) => match unsafe { op.DeleteItem(&item, None) } {
                        Ok(()) => queued.push(p),
                        Err(e) => out.failed.push((p, e.message().to_string())),
                    },
                    Err(e) => out.failed.push((p, e.message().to_string())),
                }
            }
            let perform = unsafe { op.PerformOperations() };
            out.permanently_deleted = std::mem::take(&mut *nuked.lock().unwrap());
            // Judge each item by whether it is actually gone, not by the aggregate result.
            for p in queued {
                let gone = Path::new(&p).symlink_metadata().is_err();
                if gone && !out.permanently_deleted.iter().any(|n| n.eq_ignore_ascii_case(&p)) {
                    out.recycled.push(p);
                } else if !gone {
                    let why = match &perform {
                        Err(e) if !tripped.load(Ordering::SeqCst) => e.message().to_string(),
                        _ if tripped.load(Ordering::SeqCst) => "Stopped: an earlier item could not be recycled.".to_string(),
                        _ => "The item is in use or access was denied.".to_string(),
                    };
                    out.failed.push((p, why));
                }
            }
            Ok(out)
        })
    }

    /// Open an Explorer window on the item's folder with the item selected.
    pub fn reveal_in_explorer(path: String) -> io::Result<()> {
        on_sta(move || unsafe {
            let pidl = ILCreateFromPathW(&HSTRING::from(path.as_str()));
            if pidl.is_null() {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("{path} no longer exists")));
            }
            let r = SHOpenFolderAndSelectItems(pidl, None, 0).map_err(to_io);
            ILFree(Some(pidl));
            r
        })
    }
}

#[cfg(not(windows))]
pub fn recycle(_items: Vec<RecycleItem>) -> io::Result<RecycleOutcome> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Recycle Bin is only supported on Windows"))
}

#[cfg(not(windows))]
pub fn reveal_in_explorer(_path: String) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "only supported on Windows"))
}
