//! Windows system queries: local drives and process elevation.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    /// e.g. `C:`
    pub letter: String,
    pub label: String,
    pub fs: String,
    pub total: u64,
    pub free: u64,
    /// fixed | removable | network | ram | other
    pub kind: &'static str,
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn from_wide(buf: &[u16]) -> String {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    }

    pub fn list_drives() -> Vec<Drive> {
        let mut buf = [0u16; 512];
        let n = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
        let mut out = Vec::new();
        for root in buf[..n.min(buf.len())].split(|&c| c == 0).filter(|s| !s.is_empty()) {
            let root_s = String::from_utf16_lossy(root);
            let w = wide(&root_s);
            let kind = match unsafe { GetDriveTypeW(w.as_ptr()) } {
                2 => "removable",
                3 => "fixed",
                4 => "network",
                6 => "ram",
                5 => continue, // optical
                _ => "other",
            };
            let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
            if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, &mut total, &mut total_free) } == 0 {
                continue; // no media
            }
            let mut label = [0u16; 261];
            let mut fs = [0u16; 64];
            unsafe {
                GetVolumeInformationW(
                    w.as_ptr(),
                    label.as_mut_ptr(),
                    label.len() as u32,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    fs.as_mut_ptr(),
                    fs.len() as u32,
                )
            };
            out.push(Drive {
                letter: root_s.trim_end_matches('\\').to_string(),
                label: from_wide(&label),
                fs: from_wide(&fs),
                total,
                free: total_free,
                kind,
            });
        }
        out
    }

    pub fn is_elevated() -> bool {
        let mut token: HANDLE = std::ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = unsafe {
            GetTokenInformation(token, TokenElevation, (&mut e as *mut TOKEN_ELEVATION).cast(), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len)
        };
        unsafe { CloseHandle(token) };
        ok != 0 && e.TokenIsElevated != 0
    }
}

#[cfg(windows)]
pub use win::{is_elevated, list_drives};

#[cfg(not(windows))]
pub fn list_drives() -> Vec<Drive> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}
