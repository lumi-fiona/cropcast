//! The tray's look, from Framelines: four corner marks, with an amber dot while the Virtual monitor
//! is on, white on a dark taskbar and near-black on a light one; and a native menu that follows the
//! Windows light or dark setting.

use std::cell::Cell;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCSTR, Result, w};

thread_local! {
    /// The icon the tray shows, kept alive while it does.
    static ICON: Cell<Option<HICON>> = const { Cell::new(None) };
}

/// The icon, `n`×`n` straight-alpha BGRA pixels from the top row down.
pub fn pixels(n: i32, on: bool, light: bool) -> Vec<u32> {
    let (nf, k) = (n as f32, n as f32 / 16.0);
    // The concept's 16-unit marks (1 to 6 along each edge, 1.6 thick), on whole pixels so they stay crisp.
    let (e, t, a) = (k.round(), (1.6 * k).round().max(1.0), (6.0 * k).round());
    let mark = |x: f32, y: f32| {
        let (x, y) = (x.min(nf - x), y.min(nf - y)); // every corner folded onto the top-left one
        x >= e && y >= e && ((x < e + t && y < a) || (y < e + t && x < a))
    };
    let r = 2.3 * k;
    let dot = |x: f32, y: f32| on && (x - nf / 2.0).powi(2) + (y - nf / 2.0).powi(2) < r * r;
    let (fg, amber) = if light { (0x1b1b1b, 0xd47f00) } else { (0xffffff, 0xffb43d) };
    (0..n * n)
        .map(|i| {
            // 4×4 samples a pixel; the marks and the dot never meet.
            let (px, py) = ((i % n) as f32, (i / n) as f32);
            let (mut m, mut d) = (0, 0);
            for s in 0..16 {
                let (x, y) = (px + (s % 4) as f32 / 4.0 + 0.125, py + (s / 4) as f32 / 4.0 + 0.125);
                m += mark(x, y) as u32;
                d += dot(x, y) as u32;
            }
            let (rgb, cover) = if d > 0 { (amber, d) } else { (fg, m) };
            (cover * 255 / 16) << 24 | rgb
        })
        .collect()
}

/// Whether the taskbar is light ("Choose your default Windows mode").
fn light_taskbar() -> bool {
    let (mut v, mut size) = (0u32, 4u32);
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    unsafe {
        let r = RegGetValueW(HKEY_CURRENT_USER, key, w!("SystemUsesLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as _), Some(&mut size));
        r.is_ok() && v == 1
    }
}

/// The tray icon for the Virtual monitor on or off, at the small-icon size, for the taskbar's theme.
/// It lives until the next one is made.
pub fn icon(on: bool) -> Result<HICON> {
    unsafe {
        let n = GetSystemMetrics(SM_CXSMICON);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: n,
                biHeight: -n, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let px = pixels(n, on, light_taskbar());
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
        let mask = CreateBitmap(n, n, 1, 1, None); // unused with alpha, but required
        let icon = CreateIconIndirect(&ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() });
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        let icon = icon?;
        // ponytail: the old icon goes a moment before the shell gets the new one; keep two if the tray ever blinks.
        if let Some(old) = ICON.replace(Some(icon)) {
            let _ = DestroyIcon(old);
        }
        Ok(icon)
    }
}

/// Lets the tray menu follow the Windows light or dark app setting; call again when it changes.
pub fn follow_system_theme(hwnd: HWND) {
    // ponytail: uxtheme's undocumented ordinals, as every dark tray menu does it: 135 SetPreferredAppMode
    // (1 = follow the setting), 133 AllowDarkModeForWindow, 104 RefreshImmersiveColorPolicyState,
    // 136 FlushMenuThemes. On a Windows without them the menu stays light.
    unsafe {
        let Ok(ux) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        let ordinal = |n: usize| GetProcAddress(ux, PCSTR(n as *const u8));
        if let Some(f) = ordinal(135) {
            std::mem::transmute::<_, extern "system" fn(i32) -> i32>(f)(1);
        }
        if let Some(f) = ordinal(133) {
            std::mem::transmute::<_, extern "system" fn(HWND, bool) -> bool>(f)(hwnd, true);
        }
        for n in [104, 136] {
            if let Some(f) = ordinal(n) {
                std::mem::transmute::<_, extern "system" fn()>(f)();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_four_marks_and_a_dot_when_on() {
        let alpha = |px: &[u32], x: i32, y: i32| px[(y * 16 + x) as usize] >> 24;
        let (off, on) = (pixels(16, false, false), pixels(16, true, false));
        // A crisp mark in each corner, 2 px thick; nothing between them.
        for (x, y) in [(1, 1), (14, 1), (1, 14), (14, 14), (5, 2), (2, 5)] {
            assert_eq!(alpha(&off, x, y), 255, "{x},{y}");
        }
        assert_eq!(alpha(&off, 6, 1), 0);
        assert_eq!(alpha(&off, 3, 3), 0);
        assert_eq!(alpha(&off, 8, 8), 0);
        // The dot is amber.
        assert_eq!(on[8 * 16 + 8], 0xffff_b43d);
    }

    /// Writes every tray icon over both taskbars as raw BGRA, for the eye:
    /// `CROPCAST_SHOTS=<dir> cargo test --release -- --ignored tray`, then ffmpeg it to a PNG.
    #[test]
    #[ignore]
    fn render_tray_icons() {
        let dir = std::path::PathBuf::from(std::env::var("CROPCAST_SHOTS").expect("CROPCAST_SHOTS"));
        const CELL: usize = 40;
        let (cols, rows) = (8, 2);
        let mut img = vec![0u32; cols * CELL * rows * CELL];
        for (row, (light, bg)) in [(false, 0x1c1c1c_u32), (true, 0xeeeeee)].into_iter().enumerate() {
            for (col, (n, on)) in [16, 20, 24, 32].into_iter().flat_map(|n| [(n, false), (n, true)]).enumerate() {
                let px = pixels(n, on, light);
                for y in 0..CELL {
                    for x in 0..CELL {
                        let (ix, iy) = (x as i32 - 4, y as i32 - 4);
                        let src = if (0..n).contains(&ix) && (0..n).contains(&iy) { px[(iy * n + ix) as usize] } else { 0 };
                        let a = (src >> 24) as f32 / 255.0;
                        let ch = |s: u32| (((src >> s) & 255) as f32 * a + ((bg >> s) & 255) as f32 * (1.0 - a)) as u32;
                        img[(row * CELL + y) * cols * CELL + col * CELL + x] = 0xff00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0);
                    }
                }
            }
        }
        let bytes: Vec<u8> = img.iter().flat_map(|p| p.to_le_bytes()).collect();
        std::fs::create_dir_all(dir.join("native")).unwrap();
        std::fs::write(dir.join("native").join(format!("tray-{}x{}.bgra", cols * CELL, rows * CELL)), bytes).unwrap();
    }
}
