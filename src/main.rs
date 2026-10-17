#![windows_subsystem = "windows"]

mod mirror;
mod virtual_monitor;
mod vda;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, Result, w};

use mirror::Area;
use virtual_monitor::VirtualMonitor;

const WM_TRAY: u32 = WM_APP + 1;
const WM_RECAPTURE: u32 = WM_APP + 2;

const ID_VIRTUAL_MONITOR: u32 = 1;
const ID_AUTOSTART: u32 = 2;
const ID_RESET_AREA: u32 = 3;
const ID_SHOW_BORDER: u32 = 4;
const ID_QUIT: u32 = 5;

thread_local! {
    /// Some while the Virtual monitor is on. Only the UI thread touches it, and never across a call that
    /// can re-enter the window procedure (adding a monitor sends WM_DISPLAYCHANGE).
    static VIRTUAL_MONITOR: RefCell<Option<VirtualMonitor>> = const { RefCell::new(None) };
}
static RECAPTURE_POSTED: AtomicBool = AtomicBool::new(false);
// ponytail: menu state only, the tray extras slice wires it to real behaviour.
static AUTOSTART: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn main() -> Result<()> {
    unsafe {
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
        add_tray_icon(hwnd)?;

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
    // ponytail: stock icon until the Border look ticket designs one.
    nid.hIcon = unsafe { LoadIconW(None, IDI_APPLICATION)? };
    let tip: Vec<u16> = "cropcast".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid).ok() }
}

/// An error balloon from the tray icon, for things Lumi has to know about.
fn notify_error(hwnd: HWND, title: &str, text: &str) {
    let mut nid = tray_icon(hwnd);
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_ERROR;
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

fn start_monitor(hwnd: HWND) {
    let r = virtual_monitor::real_monitor();
    let area = Area::default_for((r.right - r.left) as u32, (r.bottom - r.top) as u32);
    // Outside the borrow: adding the monitor re-enters wndproc.
    match VirtualMonitor::start(area) {
        Ok(s) => VIRTUAL_MONITOR.set(Some(s)),
        Err(e) => notify_error(hwnd, "The Virtual monitor couldn't start", &e.message()),
    }
}

fn stop_monitor() {
    // Take first, drop after: removing the monitor re-enters wndproc too.
    let s = VIRTUAL_MONITOR.take();
    drop(s);
}

unsafe fn show_menu(hwnd: HWND) -> Result<()> {
    unsafe {
        let menu = CreatePopupMenu()?;
        let checked = |on: &AtomicBool| {
            if on.load(Ordering::Relaxed) { MF_CHECKED } else { MF_UNCHECKED }
        };
        let on = if monitor_on() { MF_CHECKED } else { MF_UNCHECKED };
        AppendMenuW(menu, MF_STRING | on, ID_VIRTUAL_MONITOR as usize, w!("&Virtual monitor"))?;
        AppendMenuW(menu, MF_STRING | checked(&AUTOSTART), ID_AUTOSTART as usize, w!("Start with &Windows"))?;
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
        AppendMenuW(menu, MF_STRING, ID_RESET_AREA as usize, w!("&Reset Area to 2560×1440 center"))?;
        AppendMenuW(menu, MF_STRING, ID_SHOW_BORDER as usize, w!("Show &Border briefly"))?;
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
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
            ID_AUTOSTART => _ = AUTOSTART.fetch_xor(true, Ordering::Relaxed),
            ID_RESET_AREA | ID_SHOW_BORDER => {}
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
                // Coalesce: one add sends several of these, one fresh capture after them is enough.
                if monitor_on() && !RECAPTURE_POSTED.swap(true, Ordering::Relaxed) {
                    let _ = PostMessageW(Some(hwnd), WM_RECAPTURE, WPARAM(0), LPARAM(0));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_RECAPTURE => {
                RECAPTURE_POSTED.store(false, Ordering::Relaxed);
                let failed = VIRTUAL_MONITOR.with_borrow_mut(|s| s.as_mut().and_then(|s| s.recapture().err()));
                if let Some(e) = failed {
                    // A dead capture would freeze the stream; better off and said so.
                    stop_monitor();
                    notify_error(hwnd, "The Virtual monitor stopped", &e.message());
                }
                LRESULT(0)
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
