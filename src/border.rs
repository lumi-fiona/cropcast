//! The Border: a topmost window exactly over the Area while Edit mode is on. It's excluded from
//! capture, so viewers never see it. All of its look lives in `paint`.

use std::cell::Cell;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, Result, w};

use crate::edit::{self, Grab};
use crate::mirror::Area;

thread_local! {
    /// The last press on the Border, in desktop pixels, until Edit mode takes it.
    static PRESS: Cell<Option<POINT>> = const { Cell::new(None) };
}

pub fn take_press() -> Option<POINT> {
    PRESS.take()
}

pub struct Border {
    hwnd: HWND,
    /// The size last drawn; a move alone doesn't redraw.
    size: (i32, i32),
}

impl Border {
    pub fn new() -> Result<Border> {
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            let class = w!("cropcast-border");
            // Fails harmlessly as "already registered" from the second Edit mode on.
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance, lpszClassName: class, ..Default::default() });
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("cropcast Border"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )?;
            let border = Border { hwnd, size: (0, 0) };
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)?;
            PRESS.set(None);
            Ok(border)
        }
    }

    /// Puts the Border over `a`. The real monitor is primary, so Area pixels are desktop pixels.
    pub fn show(&mut self, a: Area) -> Result<()> {
        let size = (a.w as i32, a.h as i32);
        unsafe {
            if size == self.size {
                return SetWindowPos(self.hwnd, None, a.x, a.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
            let (w, h) = size;
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let result = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).and_then(|bmp| {
                let old = SelectObject(dc, bmp.into());
                // Alpha 1: invisible but still catches the mouse, so a drag inside the Area moves it.
                let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
                px.fill(0x0100_0000);
                paint(dc, w, h, &edit::label(a.w, a.h));
                // GDI writes alpha 0, so whatever it drew is exactly the alpha-0 pixels: make them opaque.
                let _ = GdiFlush();
                px.iter_mut().filter(|p| **p >> 24 == 0).for_each(|p| *p |= 0xff00_0000);
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                    ..Default::default()
                };
                let r = UpdateLayeredWindow(
                    self.hwnd,
                    Some(screen),
                    Some(&POINT { x: a.x, y: a.y }),
                    Some(&SIZE { cx: w, cy: h }),
                    Some(dc),
                    Some(&POINT::default()),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                SelectObject(dc, old);
                let _ = DeleteObject(bmp.into());
                r
            });
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            result?;
            self.size = size;
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            Ok(())
        }
    }
}

impl Drop for Border {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// The whole Border look, drawn with GDI onto a see-through `w`×`h` canvas; every pixel GDI
/// touches turns opaque. Restyle here.
// ponytail: plain outline, handles and size label until the Border look ticket lands.
unsafe fn paint(dc: HDC, w: i32, h: i32, label: &str) {
    const WHITE: COLORREF = COLORREF(0x00ff_ffff);
    const BLACK: COLORREF = COLORREF(0);
    const HANDLE: i32 = 10;
    let rect = |l: i32, t: i32, r: i32, b: i32| RECT { left: l, top: t, right: r, bottom: b };
    unsafe {
        let (white, black) = (CreateSolidBrush(WHITE), CreateSolidBrush(BLACK));
        // A white line with a black one inside reads on any picture.
        FrameRect(dc, &rect(0, 0, w, h), white);
        FrameRect(dc, &rect(1, 1, w - 1, h - 1), black);
        // Handles on the corners and edge midpoints.
        for (x, y) in [(0, 0), (w / 2, 0), (w, 0), (0, h / 2), (w, h / 2), (0, h), (w / 2, h), (w, h)] {
            let l = (x - HANDLE / 2).clamp(0, w - HANDLE);
            let t = (y - HANDLE / 2).clamp(0, h - HANDLE);
            FillRect(dc, &rect(l, t, l + HANDLE, t + HANDLE), black);
            FillRect(dc, &rect(l + 1, t + 1, l + HANDLE - 1, t + HANDLE - 1), white);
        }
        // The size label, centered, white on a black box.
        let font = CreateFontW(
            -28,
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let old = SelectObject(dc, font.into());
        let text: Vec<u16> = label.encode_utf16().collect();
        let mut ts = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &text, &mut ts);
        let (x, y) = ((w - ts.cx) / 2, (h - ts.cy) / 2);
        SetTextColor(dc, WHITE);
        SetBkColor(dc, BLACK);
        let bg = rect(x - 12, y - 6, x + ts.cx + 12, y + ts.cy + 6);
        let _ = ExtTextOutW(dc, x, y, ETO_OPAQUE, Some(&bg), PCWSTR(text.as_ptr()), text.len() as u32, None);
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(white.into());
        let _ = DeleteObject(black.into());
    }
}

fn cursor_for(g: Grab) -> PCWSTR {
    match g {
        (0, 0) => IDC_SIZEALL,
        (_, 0) => IDC_SIZEWE,
        (0, _) => IDC_SIZENS,
        (x, y) if x == y => IDC_SIZENWSE,
        _ => IDC_SIZENESW,
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR => {
                let (mut p, mut r) = (POINT::default(), RECT::default());
                let _ = GetCursorPos(&mut p);
                let _ = GetWindowRect(hwnd, &mut r);
                let g = edit::hit(r.right - r.left, r.bottom - r.top, p.x - r.left, p.y - r.top);
                SetCursor(LoadCursorW(None, cursor_for(g)).ok());
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                // The rest of the drag is polled: a window that never takes focus can't hold mouse capture.
                let mut r = RECT::default();
                let _ = GetWindowRect(hwnd, &mut r);
                let (x, y) = (lp.0 as i16 as i32, (lp.0 >> 16) as i16 as i32);
                PRESS.set(Some(POINT { x: r.left + x, y: r.top + y }));
                LRESULT(0)
            }
            WM_DPICHANGED => LRESULT(0), // keep the exact pixel size on any monitor
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
