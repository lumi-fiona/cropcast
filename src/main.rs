#![windows_subsystem = "windows"]

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, Result, w};

const WM_TRAY: u32 = WM_APP + 1;

const ID_SHARING: u32 = 1;
const ID_AUTOSTART: u32 = 2;
const ID_RESET_AREA: u32 = 3;
const ID_SHOW_BORDER: u32 = 4;
const ID_QUIT: u32 = 5;

// ponytail: menu state only, the build slices wire these to real behaviour.
static SHARING: AtomicBool = AtomicBool::new(false);
static AUTOSTART: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn main() -> Result<()> {
    unsafe {
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

unsafe fn show_menu(hwnd: HWND) -> Result<()> {
    unsafe {
        let menu = CreatePopupMenu()?;
        let checked = |on: &AtomicBool| {
            if on.load(Ordering::Relaxed) { MF_CHECKED } else { MF_UNCHECKED }
        };
        AppendMenuW(menu, MF_STRING | checked(&SHARING), ID_SHARING as usize, w!("&Sharing"))?;
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
            ID_SHARING => _ = SHARING.fetch_xor(true, Ordering::Relaxed),
            ID_AUTOSTART => _ = AUTOSTART.fetch_xor(true, Ordering::Relaxed),
            ID_RESET_AREA | ID_SHOW_BORDER => {}
            ID_QUIT => {
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
            m if m == TASKBAR_CREATED.load(Ordering::Relaxed) && m != 0 => {
                let _ = add_tray_icon(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
