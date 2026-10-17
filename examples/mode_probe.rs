//! Can a live Virtual monitor switch to an arbitrary size through IOCTL_UPDATE_MODE (issue #19)?
//! Windows may keep the mode list it read when the monitor arrived, and refuse anything else.
//!
//! Needs the 0.3 Driver installed. Adds a Virtual monitor for a few seconds, so the screen flickers.
//!     cargo run --release --example mode_probe

#[allow(dead_code)]
#[path = "../src/vda.rs"]
mod vda;

use std::thread::sleep;
use std::time::Duration;

use windows::Win32::Foundation::{POINT, POINTL};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::core::{PCWSTR, Result};

fn main() -> Result<()> {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)? };
    let real = unsafe {
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY), &mut mi);
        mi.rcMonitor
    };
    let mut d = vda::Display::add(2560, 1440, POINT { x: real.right, y: real.bottom })?;
    println!("Added {} at {:?}", String::from_utf16_lossy(&d.name[..d.name.len() - 1]), d.rect);

    let sizes = [(2382, 1310), (3440, 1440), (1920, 800)];
    let results: Vec<bool> = sizes.iter().map(|&(w, h)| try_size(&d, w, h)).collect();

    // cropcast's own path, back to an arbitrary size.
    print!("\n== Display::update_mode(2382, 1310): ");
    let own = d.update_mode(2382, 1310);
    match &own {
        Ok(()) => println!("ok, at {:?}", d.rect),
        Err(e) => println!("FAILED: {}", e.message()),
    }
    drop(d);
    println!("Removed the Virtual monitor.\n");

    for (&(w, h), ok) in sizes.iter().zip(&results) {
        println!("{w}x{h}: {}", if *ok { "PASS" } else { "FAIL" });
    }
    println!("update_mode: {}", if own.is_ok() { "PASS" } else { "FAIL" });
    if results.iter().all(|&ok| ok) && own.is_ok() {
        println!("RESULT: arbitrary sizes WORK; a resize keeps the same monitor.");
    } else {
        println!("RESULT: arbitrary sizes DON'T WORK; a resize falls back to remove + re-add.");
    }
    Ok(())
}

/// IOCTL, then the mode lists and ChangeDisplaySettingsExW, retried once after 1 s in case Windows
/// takes the new mode list a moment late. True if the monitor ends up exactly `w`×`h`.
fn try_size(d: &vda::Display, w: u32, h: u32) -> bool {
    println!("\n== {w}x{h}");
    if let Err(e) = d.offer_mode(w, h) {
        println!("IOCTL_UPDATE_MODE FAILED: {} (0x{:08X})", e.message(), e.code().0);
        return false;
    }
    println!("IOCTL_UPDATE_MODE ok");
    let name = PCWSTR(d.name.as_ptr());
    for (when, wait) in [("now", 0), ("after 1 s", 1000)] {
        sleep(Duration::from_millis(wait));
        for (label, flags) in [("modes", ENUM_DISPLAY_SETTINGS_FLAGS(0)), ("EDS_RAWMODE modes", EDS_RAWMODE)] {
            let modes = modes(name, flags);
            let listed = modes.iter().any(|&(mw, mh, _)| (mw, mh) == (w, h));
            let list: Vec<String> = modes.iter().map(|(mw, mh, hz)| format!("{mw}x{mh}@{hz}")).collect();
            println!("[{when}] {label}: {w}x{h} {} ({})", if listed { "LISTED" } else { "not listed" }, list.join(" "));
        }
        let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
        dm.dmPelsWidth = w;
        dm.dmPelsHeight = h;
        dm.dmDisplayFrequency = 60;
        dm.Anonymous1.Anonymous2.dmPosition = POINTL { x: d.rect.left, y: d.rect.top };
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_POSITION;
        let r = unsafe { ChangeDisplaySettingsExW(name, Some(&dm), None, CDS_UPDATEREGISTRY, None) };
        println!("[{when}] ChangeDisplaySettingsExW: {}", disp_change(r));
        if r == DISP_CHANGE_SUCCESSFUL {
            let mut cur = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
            let _ = unsafe { EnumDisplaySettingsW(name, ENUM_CURRENT_SETTINGS, &mut cur) };
            let pos = unsafe { cur.Anonymous1.Anonymous2.dmPosition };
            let (cw, ch, hz) = (cur.dmPelsWidth, cur.dmPelsHeight, cur.dmDisplayFrequency);
            println!("[{when}] now {cw}x{ch}@{hz} at ({}, {})", pos.x, pos.y);
            return (cw, ch) == (w, h);
        }
        if r != DISP_CHANGE_BADMODE {
            return false;
        }
    }
    false
}

/// Distinct (width, height, Hz) the monitor lists.
fn modes(name: PCWSTR, flags: ENUM_DISPLAY_SETTINGS_FLAGS) -> Vec<(u32, u32, u32)> {
    let mut v = Vec::new();
    let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    for i in 0.. {
        if !unsafe { EnumDisplaySettingsExW(name, ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm, flags) }.as_bool() {
            break;
        }
        let m = (dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency);
        if !v.contains(&m) {
            v.push(m);
        }
    }
    v
}

fn disp_change(r: DISP_CHANGE) -> String {
    let name = match r {
        DISP_CHANGE_SUCCESSFUL => "DISP_CHANGE_SUCCESSFUL",
        DISP_CHANGE_RESTART => "DISP_CHANGE_RESTART",
        DISP_CHANGE_FAILED => "DISP_CHANGE_FAILED",
        DISP_CHANGE_BADMODE => "DISP_CHANGE_BADMODE",
        DISP_CHANGE_NOTUPDATED => "DISP_CHANGE_NOTUPDATED",
        DISP_CHANGE_BADFLAGS => "DISP_CHANGE_BADFLAGS",
        DISP_CHANGE_BADPARAM => "DISP_CHANGE_BADPARAM",
        DISP_CHANGE_BADDUALVIEW => "DISP_CHANGE_BADDUALVIEW",
        _ => "?",
    };
    format!("{name} ({})", r.0)
}
