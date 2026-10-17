//! Windows integration, all per user (HKCU, no admin rights needed):
//!
//! - **Explorer menu** (opt-in): *Analyze with HaveSpace* on folders, drives and folder backgrounds. Windows 11
//!   lists it under *Show more options*. Re-pointed at the running exe on start if the app was moved.
//! - **Jump list** (always): the taskbar icon's jump list shows *Recent scans*; each opens the saved scan.
//! - **Tray** (opt-in): closing the window keeps HaveSpace in the notification area, where it checks the
//!   fixed drives every few minutes and shows a Windows notification when one runs low on space. Only one
//!   running instance owns the tray (a named event acts as the lock), so extra windows never add icons.
//! - **Start with Windows** (opt-in, needs the tray): starts hidden in the tray at sign-in.
//!
//! Settings live in `%LOCALAPPDATA%\HaveSpace\settings.json`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::system;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub explorer_menu: bool,
    pub tray: bool,
    pub start_with_windows: bool,
    /// Warn when a fixed drive has less than this share of its space free.
    pub low_disk_percent: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { explorer_menu: false, tray: false, start_with_windows: false, low_disk_percent: 10 }
    }
}

/// What the settings panel shows.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub settings: Settings,
    /// This instance owns the tray icon.
    pub tray_active: bool,
    /// The tray is on, but another HaveSpace window owns it.
    pub tray_elsewhere: bool,
}

pub struct Integration {
    settings: Mutex<Settings>,
    tray_owned: AtomicBool,
    #[cfg(windows)]
    tray_lock: Mutex<Option<isize>>,
    /// Drives already warned about, until they recover.
    warned: Mutex<HashSet<String>>,
    /// Started with `--tray`: stay hidden.
    pub start_hidden: bool,
}

const TRAY_ID: &str = "havespace-tray";
/// The toast sender id, registered under `HKCU\Software\Classes\AppUserModelId` so notifications work for
/// the portable exe (no Start-menu shortcut needed).
const AUMID: &str = "HaveSpace.Desktop";
const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);

fn base_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("HaveSpace")
}

fn settings_path() -> PathBuf {
    base_dir().join("settings.json")
}

fn load_settings() -> Settings {
    std::fs::read(settings_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_settings(s: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(base_dir())?;
    let tmp = settings_path().with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap())?;
    std::fs::rename(tmp, settings_path())
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Quote one command-line argument; a trailing backslash is doubled so it doesn't escape the quote.
pub fn quote_arg(a: &str) -> String {
    if a.ends_with('\\') {
        format!("\"{a}\\\"")
    } else {
        format!("\"{a}\"")
    }
}

// ---------------------------------------------------------------------------------------------
// Registry (HKCU)

#[cfg(windows)]
mod reg {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn check(code: u32) -> std::io::Result<()> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(code as i32))
        }
    }

    /// Set a string value (`name` None = the key's default value), creating the key.
    pub fn set(path: &str, name: Option<&str>, value: &str) -> std::io::Result<()> {
        let mut key: HKEY = std::ptr::null_mut();
        let p = wide(path);
        check(unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, p.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, std::ptr::null(), &mut key, std::ptr::null_mut()) })?;
        let n = name.map(wide);
        let v = wide(value);
        let r = unsafe { RegSetValueExW(key, n.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), 0, REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32) };
        unsafe { RegCloseKey(key) };
        check(r)
    }

    pub fn get(path: &str, name: Option<&str>) -> Option<String> {
        let p = wide(path);
        let n = name.map(wide);
        let mut buf = vec![0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, p.as_ptr(), n.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len)
        };
        if r != ERROR_SUCCESS {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }

    pub fn delete_tree(path: &str) -> std::io::Result<()> {
        let p = wide(path);
        match unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, p.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => Ok(()),
            r => check(r),
        }
    }

    pub fn delete_value(path: &str, name: &str) -> std::io::Result<()> {
        let (p, n) = (wide(path), wide(name));
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, p.as_ptr(), n.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => Ok(()),
            r => check(r),
        }
    }
}

/// Explorer verbs: folders, drives, and the background of an open folder (`%V` = the folder shown).
const MENU_KEYS: [&str; 3] = [r"Software\Classes\Directory\shell\HaveSpace", r"Software\Classes\Drive\shell\HaveSpace", r"Software\Classes\Directory\Background\shell\HaveSpace"];
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
fn menu_command(exe: &str) -> String {
    format!("\"{exe}\" \"%V\"")
}

#[cfg(windows)]
fn set_explorer_menu(on: bool) -> std::io::Result<()> {
    let exe = exe();
    for k in MENU_KEYS {
        if on {
            reg::set(k, Some("MUIVerb"), "Analyze with HaveSpace")?;
            reg::set(k, Some("Icon"), &format!("\"{exe}\",0"))?;
            reg::set(&format!(r"{k}\command"), None, &menu_command(&exe))?;
        } else {
            reg::delete_tree(k)?;
        }
    }
    notify_shell();
    Ok(())
}

#[cfg(windows)]
fn notify_shell() {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
}

#[cfg(windows)]
fn set_start_with_windows(on: bool) -> std::io::Result<()> {
    if on {
        reg::set(RUN_KEY, Some("HaveSpace"), &format!("\"{}\" --tray", exe()))
    } else {
        reg::delete_value(RUN_KEY, "HaveSpace")
    }
}

/// Register the toast sender so Windows shows notifications from the portable exe under "HaveSpace".
#[cfg(windows)]
fn register_aumid(app: &AppHandle) {
    let dir = base_dir();
    let icon = dir.join("notify-icon.png");
    if !icon.exists() {
        if let Some(img) = app.default_window_icon() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = write_png(&icon, img.width(), img.height(), img.rgba());
        }
    }
    let key = format!(r"Software\Classes\AppUserModelId\{AUMID}");
    let _ = reg::set(&key, Some("DisplayName"), "HaveSpace");
    if icon.exists() {
        let _ = reg::set(&key, Some("IconUri"), &icon.to_string_lossy());
    }
}

/// Minimal uncompressed (stored-deflate) PNG writer for the notification icon.
#[cfg(windows)]
fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks_exact(w as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    // zlib stream of stored blocks.
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65_535).enumerate() {
        let last = (i + 1) * 65_535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)
}

// ---------------------------------------------------------------------------------------------
// Jump list

/// Rebuild the taskbar jump list's *Recent scans* from the scan cache (on its own COM thread).
pub fn refresh_jump_list() {
    #[cfg(windows)]
    std::thread::spawn(|| {
        let items: Vec<_> = havespace_core::cache::list().into_iter().take(8).collect();
        if let Err(e) = jump_list(&items) {
            eprintln!("could not update the jump list: {e}");
        }
    });
}

#[cfg(windows)]
fn jump_list(items: &[havespace_core::cache::CacheMeta]) -> windows::core::Result<()> {
    use windows::core::{Interface, HSTRING, PROPVARIANT};
    use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
    use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PSCoerceToCanonicalValue};
    use windows::Win32::UI::Shell::{DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink};

    let exe = exe();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        let mut slots = 0u32;
        let removed: IObjectArray = list.BeginList(&mut slots)?;
        // Items the user removed from the jump list must not be added back (Windows refuses the list).
        let mut removed_args = Vec::new();
        for i in 0..removed.GetCount()? {
            if let Ok(link) = removed.GetAt::<IShellLinkW>(i) {
                let mut buf = [0u16; 1024];
                if link.GetArguments(&mut buf).is_ok() {
                    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    removed_args.push(String::from_utf16_lossy(&buf[..end]));
                }
            }
        }
        let coll: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        let mut added = 0;
        for m in items {
            if added >= slots.max(1) {
                break;
            }
            let args = format!("--saved {}", quote_arg(&m.root));
            if removed_args.contains(&args) {
                continue;
            }
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(exe.as_str()))?;
            link.SetArguments(&HSTRING::from(args.as_str()))?;
            link.SetIconLocation(&HSTRING::from(exe.as_str()), 0)?;
            link.SetDescription(&HSTRING::from(format!("Open the saved scan of {}", m.root)))?;
            let store: IPropertyStore = link.cast()?;
            let mut title = PROPVARIANT::from(m.root.as_str());
            PSCoerceToCanonicalValue(&PKEY_Title, &mut title)?;
            store.SetValue(&PKEY_Title, &title)?;
            store.Commit()?;
            coll.AddObject(&link)?;
            added += 1;
        }
        if added > 0 {
            let arr: IObjectArray = coll.cast()?;
            list.AppendCategory(&HSTRING::from("Recent scans"), &arr)?;
        }
        list.CommitList()
    }
}

// ---------------------------------------------------------------------------------------------
// Tray and low-disk warnings

#[cfg(windows)]
fn try_lock_tray() -> Option<isize> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateEventW;
    let name: Vec<u16> = "Local\\HaveSpace.Tray".encode_utf16().chain(std::iter::once(0)).collect();
    let h = unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) };
    if h.is_null() {
        return None;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(h) };
        return None;
    }
    Some(h as isize)
}

#[cfg(windows)]
fn unlock_tray(h: isize) {
    unsafe { windows_sys::Win32::Foundation::CloseHandle(h as _) };
}

impl Integration {
    pub fn new() -> Self {
        Integration {
            settings: Mutex::new(load_settings()),
            tray_owned: AtomicBool::new(false),
            #[cfg(windows)]
            tray_lock: Mutex::new(None),
            warned: Mutex::new(HashSet::new()),
            start_hidden: std::env::args().any(|a| a == "--tray"),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn tray_active(&self) -> bool {
        self.tray_owned.load(Ordering::Relaxed)
    }

    pub fn info(&self) -> Info {
        let settings = self.settings();
        let tray_active = self.tray_active();
        Info { tray_elsewhere: settings.tray && !tray_active, settings, tray_active }
    }
}

/// Show and focus the main window.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg(windows)]
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let open = MenuItem::with_id(app, "open", "Open HaveSpace", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Check free space now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit HaveSpace", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &check, &PredefinedMenuItem::separator(app)?, &quit])?;
    let mut b = TrayIconBuilder::with_id(TRAY_ID).tooltip("HaveSpace").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.on_menu_event(|app, e| match e.id.as_ref() {
        "open" => show_main(app),
        "check" => check_drives(app, true),
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|tray, e| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
}

/// Create or remove this instance's tray icon to match the settings. Called on the main thread.
#[cfg(windows)]
fn sync_tray(app: &AppHandle) {
    let integ = app.state::<Integration>();
    let want = integ.settings().tray;
    let mut lock = integ.tray_lock.lock().unwrap();
    if want && lock.is_none() {
        if let Some(h) = try_lock_tray() {
            match build_tray(app) {
                Ok(()) => {
                    *lock = Some(h);
                    integ.tray_owned.store(true, Ordering::Relaxed);
                    drop(lock);
                    check_drives(app, false);
                }
                Err(e) => {
                    eprintln!("could not create the tray icon: {e}");
                    unlock_tray(h);
                }
            }
        }
    } else if !want {
        if let Some(h) = lock.take() {
            let _ = app.remove_tray_by_id(TRAY_ID);
            unlock_tray(h);
            integ.tray_owned.store(false, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPath {
    pub path: String,
    /// Open the saved scan (if any) instead of scanning again.
    pub saved: bool,
}

/// Check the fixed drives; warn (once, until it recovers) about any below the threshold. `always`: also
/// say so when everything is fine (the tray's "Check free space now").
#[cfg(windows)]
fn check_drives(app: &AppHandle, always: bool) {
    let integ = app.state::<Integration>();
    let pct = integ.settings().low_disk_percent.clamp(1, 50) as f64;
    let drives: Vec<_> = system::list_drives().into_iter().filter(|d| d.kind == "fixed" && d.total > 0).collect();
    let mut tip = String::from("HaveSpace");
    let mut low = Vec::new();
    {
        let mut warned = integ.warned.lock().unwrap();
        for d in &drives {
            let free_pct = d.free as f64 * 100.0 / d.total as f64;
            tip.push_str(&format!("\n{} {} free ({:.0}%)", d.letter, havespace_core::format_bytes(d.free), free_pct));
            if free_pct < pct {
                if warned.insert(d.letter.clone()) || always {
                    low.push((d.clone(), free_pct));
                }
            } else if free_pct >= pct + 2.0 {
                warned.remove(&d.letter);
            }
        }
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tip.chars().take(127).collect::<String>()));
    }
    for (d, free_pct) in low {
        let root = format!("{}\\", d.letter);
        let label = if d.label.is_empty() { String::new() } else { format!(" ({})", d.label) };
        show_toast(
            app,
            &format!("Drive {}{label} is almost full", d.letter),
            &format!("{} free, {:.0}% of {}. Click to see what's taking the space.", havespace_core::format_bytes(d.free), free_pct, havespace_core::format_bytes(d.total)),
            Some(root),
        );
    }
    if always && drives.iter().all(|d| d.free as f64 * 100.0 / d.total as f64 >= pct) {
        show_toast(app, "Plenty of space", &format!("Every fixed drive has at least {pct:.0}% free."), None);
    }
}

/// A Windows notification; clicking it opens HaveSpace (on `open`, scanning that drive).
#[cfg(windows)]
pub fn show_toast(app: &AppHandle, title: &str, text: &str, open: Option<String>) {
    use tauri_winrt_notification::Toast;
    let make = |id: &str| {
        let app = app.clone();
        let open = open.clone();
        Toast::new(id).title(title).text1(text).on_activated(move |_| {
            let app2 = app.clone();
            let open = open.clone();
            let _ = app.run_on_main_thread(move || {
                show_main(&app2);
                if let Some(path) = open {
                    let saved = havespace_core::cache::list().iter().any(|m| m.root.eq_ignore_ascii_case(&path));
                    let _ = app2.emit("open-path", OpenPath { path, saved });
                }
            });
            Ok(())
        })
    };
    if let Err(e) = make(AUMID).show() {
        eprintln!("notification failed ({e}); retrying with the PowerShell sender");
        let _ = make(Toast::POWERSHELL_APP_ID).show();
    }
}

/// Apply new settings: registry entries, tray, then save. Errors leave the earlier settings in place.
pub fn apply(app: &AppHandle, mut new: Settings) -> Result<Info, String> {
    new.low_disk_percent = new.low_disk_percent.clamp(1, 50);
    if !new.tray {
        new.start_with_windows = false;
    }
    let integ = app.state::<Integration>();
    let old = integ.settings();
    #[cfg(windows)]
    {
        if new.explorer_menu != old.explorer_menu {
            set_explorer_menu(new.explorer_menu).map_err(|e| format!("Couldn't change the Explorer menu: {e}"))?;
        }
        if new.start_with_windows != old.start_with_windows {
            set_start_with_windows(new.start_with_windows).map_err(|e| format!("Couldn't change Start with Windows: {e}"))?;
        }
    }
    if new.low_disk_percent != old.low_disk_percent {
        integ.warned.lock().unwrap().clear();
    }
    save_settings(&new).map_err(|e| format!("Couldn't save the settings: {e}"))?;
    *integ.settings.lock().unwrap() = new;
    #[cfg(windows)]
    {
        let app2 = app.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = app.run_on_main_thread(move || {
            sync_tray(&app2);
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(Duration::from_secs(3));
    }
    Ok(integ.info())
}

/// At startup: re-point registry entries at this exe if it moved, register the toast sender, take the
/// tray if it's on and free, and start the free-space monitor.
pub fn init(app: &AppHandle) {
    #[cfg(windows)]
    {
        let s = app.state::<Integration>().settings();
        let exe = exe();
        if s.explorer_menu && reg::get(&format!(r"{}\command", MENU_KEYS[0]), None).as_deref() != Some(menu_command(&exe).as_str()) {
            let _ = set_explorer_menu(true);
        }
        if s.start_with_windows && reg::get(RUN_KEY, Some("HaveSpace")).as_deref() != Some(format!("\"{exe}\" --tray").as_str()) {
            let _ = set_start_with_windows(true);
        }
        register_aumid(app);
        sync_tray(app);
        refresh_jump_list();

        // Monitor: check drives every few minutes while this instance owns the tray, and take the tray
        // over if the instance that had it quit.
        let app = app.clone();
        std::thread::spawn(move || {
            let mut last = Instant::now();
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let integ = app.state::<Integration>();
                if integ.settings().tray && !integ.tray_active() {
                    let a = app.clone();
                    let _ = app.run_on_main_thread(move || sync_tray(&a));
                }
                if integ.tray_active() && last.elapsed() >= CHECK_EVERY {
                    last = Instant::now();
                    check_drives(&app, false);
                }
            }
        });
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// Send the low-disk notification for the fullest drive now, so the user can see what it looks like.
pub fn test_notification(app: &AppHandle) {
    #[cfg(windows)]
    {
        let drives: Vec<_> = system::list_drives().into_iter().filter(|d| d.kind == "fixed" && d.total > 0).collect();
        let Some(d) = drives.iter().min_by(|a, b| (a.free as f64 / a.total as f64).total_cmp(&(b.free as f64 / b.total as f64))) else { return };
        show_toast(
            app,
            &format!("Test: drive {} has {} free", d.letter, havespace_core::format_bytes(d.free)),
            "This is how HaveSpace warns you when a drive runs low. Click to scan it.",
            Some(format!("{}\\", d.letter)),
        );
    }
    #[cfg(not(windows))]
    let _ = app;
}
