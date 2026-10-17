//! Virtual monitor on: it exists and a topmost window on it shows the Area at 60 fps.
//! Dropping `VirtualMonitor` undoes it all, in order: capture, presenter, monitor.

use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

use crate::mirror::{self, Area, Capture, Gpu, Mirror};
use crate::vda::Display;

pub struct VirtualMonitor {
    // Fields drop top to bottom; see the module doc.
    capture: Option<Capture>,
    mirror: Arc<Mutex<Mirror>>,
    presenter: Presenter,
    gpu: Gpu,
    /// None only for the moment a resize swaps it.
    display: Option<Display>,
}

impl VirtualMonitor {
    pub fn start(area: Area) -> Result<VirtualMonitor> {
        let real = real_monitor();
        // Touch the real monitor only at its bottom-right corner, so the cursor can't wander over.
        let display = Display::add(area.w, area.h, POINT { x: real.right, y: real.bottom })?;
        let gpu = mirror::create_device()?;
        let presenter = Presenter::new(display.rect)?;
        let mirror = Arc::new(Mutex::new(Mirror::new(&gpu, presenter.0, area)?));
        let mut vm = VirtualMonitor { capture: None, mirror, presenter, gpu, display: Some(display) };
        vm.recapture()?;
        Ok(vm)
    }

    /// Changes the Virtual monitor's mode to `area`'s size in place, so Discord keeps sharing it,
    /// keeping the presenter, device and swapchain.
    pub fn resize(&mut self, area: Area) -> Result<()> {
        self.capture = None;
        if !self.display.as_mut().is_some_and(|d| d.update_mode(area.w, area.h).is_ok()) {
            // ponytail: Windows may refuse a size it didn't list when the monitor arrived. Then this
            // re-adds the monitor at the new size, which pauses Discord's share. Delete once
            // examples/mode_probe.rs shows arbitrary sizes work live.
            self.mirror.lock().unwrap().presenting = false;
            unsafe {
                // Hide before the monitor goes, or Windows moves the window onto the real screen.
                let _ = ShowWindow(self.presenter.0, SW_HIDE);
            }
            self.display = None;
            let real = real_monitor();
            self.display = Some(Display::add(area.w, area.h, POINT { x: real.right, y: real.bottom })?);
        }
        self.mirror.lock().unwrap().resize(area)?;
        let r = self.display.as_ref().unwrap().rect;
        unsafe {
            SetWindowPos(
                self.presenter.0,
                Some(HWND_TOPMOST),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
        }
        self.mirror.lock().unwrap().presenting = true;
        self.recapture()
    }

    /// Points the copy box at `area` if it has the Virtual monitor's size (a live move); false if
    /// it needs a `resize`.
    pub fn set_area(&self, area: Area) -> bool {
        let mut m = self.mirror.lock().unwrap();
        let fits = (m.area.w, m.area.h) == (area.w, area.h);
        if fits {
            m.area = area;
        }
        fits
    }

    /// Any monitor add, remove or mode change kills the WGC capture of the real monitor and
    /// its HMONITOR, so start a fresh one.
    pub fn recapture(&mut self) -> Result<()> {
        self.capture = None;
        let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        self.capture = Some(mirror::start_capture(&self.gpu, monitor, self.mirror.clone())?);
        Ok(())
    }
}

impl Drop for VirtualMonitor {
    fn drop(&mut self) {
        self.capture = None;
        self.mirror.lock().unwrap().presenting = false;
        // Hide before the monitor goes, or Windows moves the window onto the real screen.
        unsafe {
            let _ = ShowWindow(self.presenter.0, SW_HIDE);
        }
    }
}

/// The real (primary) monitor's rect in desktop pixels.
pub fn real_monitor() -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut mi);
        mi.rcMonitor
    }
}

/// The borderless, topmost, never-focused window covering the Virtual monitor. Discord shares
/// that screen, so this window must stay capturable (no WDA_EXCLUDEFROMCAPTURE).
struct Presenter(HWND);

impl Presenter {
    fn new(r: RECT) -> Result<Presenter> {
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            let class = w!("cropcast-presenter");
            // Fails harmlessly as "already registered" on the second Virtual monitor on.
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance, lpszClassName: class, ..Default::default() });
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("cropcast"),
                WS_POPUP,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                None,
                None,
                Some(instance),
                None,
            )?;
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(Presenter(hwnd))
        }
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DPICHANGED => LRESULT(0), // keep the exact pixel size on any monitor
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
