//! Which paths DiskLens refuses to send to the Recycle Bin, and which get an extra warning.
//!
//! This is a safety net on top of the confirmation dialog: deleting these by hand breaks Windows or
//! apps, and the right way to reclaim their space is a Windows setting or an uninstaller.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "lowercase")]
pub enum Protection {
    /// Fine to send to the Recycle Bin after confirmation.
    None,
    /// Allowed, but the confirmation dialog shows this warning.
    Warn(&'static str),
    /// Never allowed.
    Blocked(&'static str),
}

fn components(path: &str) -> Vec<String> {
    path.trim_end_matches('\\').split('\\').filter(|s| !s.is_empty()).map(|s| s.to_ascii_lowercase()).collect()
}

/// Classify an absolute Windows path such as `C:\Users\me\Downloads\file.zip`.
pub fn protection(path: &str) -> Protection {
    let c = components(path);
    let Some(first) = c.first() else { return Protection::Blocked("Not a valid path.") };
    if !(first.len() == 2 && first.ends_with(':')) {
        // UNC / network paths: allowed, but network shares usually have no Recycle Bin.
        return Protection::Warn("Network locations often have no Recycle Bin. DiskLens will refuse if the item can't be recycled.");
    }
    if c.len() == 1 {
        return Protection::Blocked("This is the root of a drive.");
    }
    let top = c[1].as_str();
    let depth = c.len() - 1; // components below the drive root
    match top {
        "windows" => return Protection::Blocked("Part of Windows. Use Disk Cleanup or Storage settings to reclaim space here."),
        "system volume information" | "$recycle.bin" | "recovery" | "boot" | "$winreagent" | "$windows.~bt" | "$windows.~ws" | "$sysreset" => {
            return Protection::Blocked("Managed by Windows. Use the matching Windows setting to reclaim this space.")
        }
        "pagefile.sys" | "hiberfil.sys" | "swapfile.sys" | "bootmgr" | "dumpstack.log.tmp" if depth == 1 => {
            return Protection::Blocked("Used by Windows while it runs. Change it through Windows settings instead.")
        }
        "program files" | "program files (x86)" | "programdata" if depth == 1 => {
            return Protection::Blocked("A Windows system folder. Remove individual apps from Settings › Apps instead.")
        }
        "program files" | "program files (x86)" => {
            return Protection::Warn("Installed apps live here. Uninstalling from Settings › Apps is safer than deleting files.")
        }
        "programdata" => return Protection::Warn("Shared app data. Apps may break if their files here disappear."),
        "users" if depth <= 2 => return Protection::Blocked("A user profile folder. Delete the files inside it instead."),
        _ => {}
    }
    if top == "users" && depth == 3 && c[3] == "appdata" {
        return Protection::Blocked("Holds settings for every app you use.");
    }
    if top == "users" && c.get(3).is_some_and(|s| s == "appdata") {
        return Protection::Warn("App data: programs may lose settings or caches. Close the app first.");
    }
    Protection::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_system_locations() {
        for p in [
            r"C:\",
            r"C:\Windows",
            r"C:\Windows\WinSxS\amd64_x\file.dll",
            r"c:\WINDOWS\temp",
            r"C:\pagefile.sys",
            r"C:\System Volume Information",
            r"C:\$Recycle.Bin\S-1-5",
            r"C:\Program Files",
            r"C:\ProgramData",
            r"C:\Users",
            r"C:\Users\me",
            r"C:\Users\me\AppData",
        ] {
            assert!(matches!(protection(p), Protection::Blocked(_)), "{p} should be blocked");
        }
    }

    #[test]
    fn warns_for_app_locations() {
        for p in [r"C:\Program Files\Adobe", r"C:\ProgramData\Package Cache", r"C:\Users\me\AppData\Local\Temp", r"\\server\share\x"] {
            assert!(matches!(protection(p), Protection::Warn(_)), "{p} should warn");
        }
    }

    #[test]
    fn allows_ordinary_files() {
        for p in [r"C:\Users\me\Downloads\big.iso", r"D:\Games\old", r"C:\pagefile.sys.bak\x", r"C:\Data\Windows\notes.txt"] {
            assert_eq!(protection(p), Protection::None, "{p}");
        }
    }
}
