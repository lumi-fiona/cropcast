#![windows_subsystem = "windows"]

mod autostart;
mod border;
mod driver;
mod edit;
mod mirror;
mod settings;
mod tray_look;
mod virtual_monitor;
mod vda;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFY_ICON_INFOTIP_FLAGS, NOTIFYICONDATAW, QUNS_RUNNING_D3D_FULL_SCREEN,
    SHQueryUserNotificationState, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, Result, w};

use mirror::Area;
use virtual_monitor::VirtualMonitor;

const WM_TRAY: u32 = WM_APP + 1;
const WM_RECAPTURE: u32 = WM_APP + 2;
/// From a second launch: show the Border briefly, as a "here I am".
const WM_HELLO: u32 = WM_APP + 3;

const ID_VIRTUAL_MONITOR: u32 = 1;
const ID_AUTOSTART: u32 = 2;
const ID_RESET_AREA: u32 = 3;
const ID_SHOW_BORDER: u32 = 4;
const ID_QUIT: u32 = 5;
const ID_REMOVE_DRIVER: u32 = 6;

const ID_POLL: usize = 1;
/// Hotkey poll interval, and the faster one while Edit mode follows the mouse.
const POLL_IDLE_MS: u32 = 50;
const POLL_EDIT_MS: u32 = 15; // one 15.6 ms system tick, about 60 Hz
const ID_FLASH: usize = 2;
const FLASH_MS: u32 = 1500;
const ID_FULLSCREEN: usize = 3;
const FULLSCREEN_MS: u32 = 2000;

thread_local! {
    /// Some while the Virtual monitor is on. Only the UI thread touches it, and never across a call that
    /// can re-enter the window procedure (adding a monitor sends WM_DISPLAYCHANGE).
    static VIRTUAL_MONITOR: RefCell<Option<VirtualMonitor>> = const { RefCell::new(None) };
    /// The Area, kept while the Virtual monitor is off too. Edit mode changes it on release.
    static AREA: Cell<Area> = Cell::new(saved_area());
    /// The real monitor's size the Area was last fitted to. Our own adds and mode changes never
    /// change it, so a new size means the real monitor itself changed.
    static REAL: Cell<(i32, i32)> = const { Cell::new((0, 0)) }; // set at startup
    /// Some while Edit mode is on.
    static EDIT: RefCell<Option<edit::Session>> = const { RefCell::new(None) };
    /// Some while Show Border briefly shows it.
    static FLASH: RefCell<Option<border::Border>> = const { RefCell::new(None) };
}
static RECAPTURE_POSTED: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// Whether the tray icon shows the Virtual monitor on.
static ICON_ON: AtomicBool = AtomicBool::new(false);
/// The exclusive-fullscreen warning shows once per run.
static FULLSCREEN_SAID: AtomicBool = AtomicBool::new(false);

fn main() -> Result<()> {
    if let Some(code) = driver::run_flag() {
        std::process::exit(code);
    }
    unsafe {
        // One cropcast per session. The handle lives as long as the process.
        let _single = CreateMutexW(None, false, w!(r"Local\cropcast"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(first) = FindWindowW(w!("cropcast"), w!("cropcast")) {
                let _ = PostMessageW(Some(first), WM_HELLO, WPARAM(0), LPARAM(0));
            }
            return Ok(());
        }
        // Physical pixels everywhere: the Area, the Virtual monitor and the presenter must match 1:1.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        RoInitialize(RO_INIT_MULTITHREADED)?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("cropcast");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        });
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        // A hidden top-level window, not HWND_MESSAGE: message-only windows miss the
        // TaskbarCreated broadcast, so the icon would vanish after Explorer restarts.
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("cropcast"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        tray_look::follow_system_theme(hwnd);
        add_tray_icon(hwnd)?;
        REAL.set(real_size());
        SetTimer(Some(hwnd), ID_POLL, POLL_IDLE_MS, None);
        SetTimer(Some(hwnd), ID_FULLSCREEN, FULLSCREEN_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}

fn tray_icon(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

unsafe fn add_tray_icon(hwnd: HWND) -> Result<()> {
    let mut nid = tray_icon(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    let on = monitor_on();
    ICON_ON.store(on, Ordering::Relaxed);
    nid.hIcon = tray_look::icon(on)?;
    let tip: Vec<u16> = "cropcast".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid).ok() }
}

/// Swaps the tray icon when the Virtual monitor turns on or off, or always on `restyle` (a theme change).
fn update_tray_icon(hwnd: HWND, restyle: bool) {
    let on = monitor_on();
    if ICON_ON.swap(on, Ordering::Relaxed) == on && !restyle {
        return;
    }
    let Ok(icon) = tray_look::icon(on) else { return };
    let mut nid = tray_icon(hwnd);
    nid.uFlags = NIF_ICON;
    nid.hIcon = icon;
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// A balloon from the tray icon, for things Lumi has to know about.
fn notify(hwnd: HWND, icon: NOTIFY_ICON_INFOTIP_FLAGS, title: &str, text: &str) {
    let mut nid = tray_icon(hwnd);
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = icon;
    let copy = |dst: &mut [u16], s: &str| {
        let src: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
        dst[..src.len()].copy_from_slice(&src);
    };
    copy(&mut nid.szInfoTitle, title);
    copy(&mut nid.szInfo, text);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn monitor_on() -> bool {
    VIRTUAL_MONITOR.with_borrow(|s| s.is_some())
}

fn real_size() -> (i32, i32) {
    let r = virtual_monitor::real_monitor();
    (r.right - r.left, r.bottom - r.top)
}

fn default_area() -> Area {
    let (w, h) = real_size();
    Area::default_for(w as u32, h as u32)
}

/// The saved Area moved onto the real monitor, or the default if it doesn't fit. From the file, not
/// AREA, so the Area a lower resolution shrank comes back with the higher one.
fn saved_area() -> Area {
    settings::load().unwrap_or_else(default_area)
}

fn start_monitor(hwnd: HWND) {
    if driver::needed()
        && let Err(e) = driver::install()
    {
        notify(hwnd, NIIF_ERROR, "The Virtual monitor stays off", &e.message());
        return;
    }
    // Outside the borrow: adding the monitor re-enters wndproc.
    match VirtualMonitor::start(AREA.get()) {
        Ok(s) => VIRTUAL_MONITOR.set(Some(s)),
        Err(e) => notify(hwnd, NIIF_ERROR, "The Virtual monitor couldn't start", &e.message()),
    }
}

fn stop_monitor() {
    // Take first, drop after: removing the monitor re-enters wndproc too.
    let s = VIRTUAL_MONITOR.take();
    drop(s);
}

/// Enters, drives and leaves Edit mode from the hotkey poll.
fn tick(hwnd: HWND) {
    let held = edit::hotkey_held();
    let editing = EDIT.with_borrow(|e| e.is_some());
    if held && !editing {
        // ponytail: a Border that can't be made means no Edit mode; the next tick tries again.
        let Ok(s) = edit::Session::begin(AREA.get()) else { return };
        drop(FLASH.take());
        EDIT.set(Some(s));
        unsafe { SetTimer(Some(hwnd), ID_POLL, POLL_EDIT_MS, None) };
    } else if held {
        // A move is live; a new size waits for the release.
        if let Some(area) = EDIT.with_borrow_mut(|e| e.as_mut().and_then(|s| s.update())) {
            VIRTUAL_MONITOR.with_borrow(|v| v.as_ref().map(|v| v.set_area(area)));
        }
    } else if let Some(s) = EDIT.take() {
        let changed = s.area != AREA.get();
        AREA.set(s.area);
        drop(s); // hides the Border
        if changed {
            save_area(hwnd);
        }
        apply_area(hwnd, false);
        unsafe { SetTimer(Some(hwnd), ID_POLL, POLL_IDLE_MS, None) };
    }
}

/// Puts the Virtual monitor, if on, on the Area. A resize changes its mode, which restarts WGC too.
/// `replace` resizes even at the same size, to put it back at the real monitor's corner.
fn apply_area(hwnd: HWND, replace: bool) {
    if !VIRTUAL_MONITOR.with_borrow(|v| v.as_ref().is_none_or(|v| !replace && v.set_area(AREA.get()))) {
        // Out of the cell while resizing: a mode change re-enters wndproc.
        if let Some(mut v) = VIRTUAL_MONITOR.take() {
            match v.resize(AREA.get()) {
                Ok(()) => VIRTUAL_MONITOR.set(Some(v)),
                Err(e) => {
                    drop(v);
                    notify(hwnd, NIIF_ERROR, "The Virtual monitor stopped", &e.message());
                }
            }
        }
    }
}

/// Coalesces: one add sends several WM_DISPLAYCHANGEs, and one look after them is enough.
fn post_recapture(hwnd: HWND) {
    if !RECAPTURE_POSTED.swap(true, Ordering::Relaxed) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_RECAPTURE, WPARAM(0), LPARAM(0));
        }
    }
}

/// After display changes, ours included: refits the Area when the real monitor changed size, and
/// restarts the capture, or turns the Virtual monitor off once Windows has dropped it.
fn display_changed(hwnd: HWND) {
    let size = real_size();
    let resized = REAL.replace(size) != size;
    if resized {
        // Not saved: the file keeps the Area chosen at the old resolution, for when it comes back.
        AREA.set(saved_area());
    }
    let failed = if VIRTUAL_MONITOR.with_borrow(|v| v.as_ref().is_some_and(|v| !v.alive())) {
        // Its presenter would land on the real monitor, covering it with a mirror of itself.
        Some(if vda::ready() {
            "Windows removed it, or the real monitor went away. Turn it on again from the tray.".to_string()
        } else {
            "The Driver went away. Turning the Virtual monitor on installs it again.".to_string()
        })
    } else if resized {
        apply_area(hwnd, true); // says so itself if it fails
        None
    } else {
        VIRTUAL_MONITOR.with_borrow_mut(|s| s.as_mut().and_then(|s| s.recapture().err())).map(|e| e.message())
    };
    if let Some(why) = failed {
        // A dead capture would freeze the stream; better off and said so.
        stop_monitor();
        notify(hwnd, NIIF_ERROR, "The Virtual monitor stopped", &why);
    }
}

fn save_area(hwnd: HWND) {
    if let Err(e) = settings::save(AREA.get()) {
        notify(hwnd, NIIF_ERROR, "The Area couldn't be saved", &e.to_string());
    }
}

/// The Border for FLASH_MS, click-through, outside Edit mode (which shows its own).
fn show_border_briefly(hwnd: HWND) {
    if EDIT.with_borrow(|e| e.is_some()) {
        return;
    }
    let Ok(mut b) = border::Border::new(virtual_monitor::real_monitor()) else { return };
    b.pass_clicks();
    if b.show(AREA.get(), None, None).is_ok() {
        drop(FLASH.replace(Some(b)));
        unsafe { SetTimer(Some(hwnd), ID_FLASH, FLASH_MS, None) };
    }
}

unsafe fn show_menu(hwnd: HWND) -> Result<()> {
    unsafe {
        let menu = CreatePopupMenu()?;
        let checked = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        AppendMenuW(menu, MF_STRING | checked(monitor_on()), ID_VIRTUAL_MONITOR as usize, w!("&Virtual monitor"))?;
        AppendMenuW(menu, MF_STRING | checked(autostart::enabled()), ID_AUTOSTART as usize, w!("Start with &Windows"))?;
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, ID_RESET_AREA as usize, w!("&Reset Area to 2560×1440 center"))?;
        AppendMenuW(menu, MF_STRING, ID_SHOW_BORDER as usize, w!("Show &Border briefly"))?;
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, ID_REMOVE_DRIVER as usize, w!("Remove &driver…"))?;
        AppendMenuW(menu, MF_STRING, ID_QUIT as usize, w!("&Quit"))?;

        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor)?;
        // Without this the menu won't close when clicking elsewhere (KB135788).
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            cursor.x,
            cursor.y,
            None,
            hwnd,
            None,
        )
        .0 as u32;
        DestroyMenu(menu)?;

        match id {
            ID_VIRTUAL_MONITOR if monitor_on() => stop_monitor(),
            ID_VIRTUAL_MONITOR => start_monitor(hwnd),
            ID_AUTOSTART => {
                if let Err(e) = autostart::set(!autostart::enabled()) {
                    notify(hwnd, NIIF_ERROR, "Start with Windows couldn't be changed", &e.message());
                }
            }
            ID_RESET_AREA => {
                AREA.set(default_area());
                save_area(hwnd);
                apply_area(hwnd, false);
                show_border_briefly(hwnd);
            }
            ID_SHOW_BORDER => show_border_briefly(hwnd),
            ID_REMOVE_DRIVER => {
                stop_monitor();
                match driver::uninstall() {
                    Ok(()) => notify(hwnd, NIIF_INFO, "Driver removed", "Turning the Virtual monitor on installs it again."),
                    Err(e) => notify(hwnd, NIIF_ERROR, "The Driver is still installed", &e.message()),
                }
            }
            ID_QUIT => {
                stop_monitor();
                let _ = Shell_NotifyIconW(NIM_DELETE, &tray_icon(hwnd));
                PostQuitMessage(0);
            }
            _ => {}
        }
        Ok(())
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY if matches!(lparam.0 as u32, WM_RBUTTONUP | WM_LBUTTONUP) => {
                let _ = show_menu(hwnd);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                post_recapture(hwnd);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_POWERBROADCAST if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC => {
                // Back from sleep: check the Virtual monitor survived, and restart the capture.
                post_recapture(hwnd);
                LRESULT(1)
            }
            WM_RECAPTURE => {
                RECAPTURE_POSTED.store(false, Ordering::Relaxed);
                display_changed(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == ID_FULLSCREEN => {
                // Discord's own test for a game WGC can't see. Windows also says yes for a game it
                // quietly runs borderless (fullscreen optimizations), which mirrors fine, hence "may".
                if monitor_on()
                    && SHQueryUserNotificationState().is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN)
                    && !FULLSCREEN_SAID.swap(true, Ordering::Relaxed)
                {
                    notify(
                        hwnd,
                        NIIF_WARNING,
                        "A game is in exclusive fullscreen",
                        "Viewers may see black or a frozen picture. Set the game to borderless or windowed to share it.",
                    );
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == ID_FLASH => {
                let _ = KillTimer(Some(hwnd), ID_FLASH);
                drop(FLASH.take());
                LRESULT(0)
            }
            WM_TIMER => {
                tick(hwnd);
                // Catches every way the Virtual monitor turns on or off.
                update_tray_icon(hwnd, false);
                LRESULT(0)
            }
            WM_HELLO => {
                show_border_briefly(hwnd);
                LRESULT(0)
            }
            WM_SETTINGCHANGE if lparam.0 != 0 && PCWSTR(lparam.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet") => {
                // Light or dark mode changed: the menu and the icon follow.
                tray_look::follow_system_theme(hwnd);
                update_tray_icon(hwnd, true);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_ENDSESSION if wparam.0 != 0 => {
                stop_monitor();
                LRESULT(0)
            }
            m if m == TASKBAR_CREATED.load(Ordering::Relaxed) && m != 0 => {
                let _ = add_tray_icon(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
