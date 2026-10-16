//! cropcast end-to-end spike (issue #6): SudoVDA Virtual monitor + WGC mirror of the Area.
//! Throwaway. Usage: cropcast-spike [W H] [--preview]

mod mirror;
mod vda;

use std::io::BufRead;
use std::mem::size_of;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mirror::{Area, Mirror};
use vda::{Display, Vda};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Console::{CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, Result, w};

const HELP: &str = "commands: m X Y (move Area) | r W H (resize) | off | on | s (stats) | q (quit)";

static MAIN_TID: AtomicU32 = AtomicU32::new(0);
const WM_RECAPTURE: u32 = WM_APP + 1;

fn recapture() {
    unsafe {
        let _ = PostThreadMessageW(MAIN_TID.load(Ordering::SeqCst), WM_RECAPTURE, WPARAM(0), LPARAM(0));
    }
}
static CLEANED_UP: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {}", e.message());
            1
        }
    };
    std::process::exit(code);
}

fn usage() -> ! {
    eprintln!(
        "usage: cropcast-spike [W H] [--preview]\n  default Area: 2560x1440 centered (--preview: 640x360 centered)\n  {HELP}"
    );
    std::process::exit(2);
}

struct Real {
    monitor: HMONITOR,
    rect: RECT,
}

impl Real {
    fn size(&self) -> (u32, u32) {
        (
            (self.rect.right - self.rect.left) as u32,
            (self.rect.bottom - self.rect.top) as u32,
        )
    }
}

fn primary_monitor() -> Real {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let _ = GetMonitorInfoW(monitor, &mut mi.monitorInfo);
        let mut dm = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let _ = EnumDisplaySettingsW(PCWSTR(mi.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm);
        let r = mi.monitorInfo.rcMonitor;
        println!(
            "[real] primary monitor {}: {}x{} at ({},{}) {} Hz",
            String::from_utf16_lossy(&mi.szDevice).trim_end_matches('\0'),
            r.right - r.left,
            r.bottom - r.top,
            r.left,
            r.top,
            dm.dmDisplayFrequency
        );
        Real { monitor, rect: r }
    }
}

/// Parses "W H" and checks the Area fits the real monitor.
fn parse_size(a: &str, b: &str, (mw, mh): (u32, u32)) -> std::result::Result<(u32, u32), String> {
    let (w, h) = match (a.parse::<u32>(), b.parse::<u32>()) {
        (Ok(w), Ok(h)) => (w, h),
        _ => return Err(format!("not a size: {a} {b}")),
    };
    if w == 0 || h == 0 || w > mw || h > mh {
        return Err(format!(
            "size must be 1..={mw} x 1..={mh} (the real monitor), got {w}x{h}"
        ));
    }
    Ok((w, h))
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let preview = args.iter().any(|a| a == "--preview");
    let nums: Vec<&String> = args.iter().filter(|a| *a != "--preview").collect();
    if !matches!(nums.len(), 0 | 2) {
        usage();
    }

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let real = primary_monitor();
    let (rw, rh) = real.size();
    let (w, h) = match nums.as_slice() {
        [a, b] => parse_size(a, b, (rw, rh)).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            usage()
        }),
        _ if preview => (640, 360),
        _ => (2560.min(rw), 1440.min(rh)),
    };
    let area = Area {
        x: (rw as i32 - w as i32) / 2,
        y: (rh as i32 - h as i32) / 2,
        w,
        h,
    };
    println!(
        "[area] {}x{} at ({},{}){}",
        area.w,
        area.h,
        area.x,
        area.y,
        if preview {
            "  (preview mode, no driver)"
        } else {
            ""
        }
    );

    // Driver first: fail fast before any window or capture exists.
    let display = if preview {
        None
    } else {
        let vda = Vda::open()?;
        vda.check_version()?;
        let (timeout, countdown) = vda.watchdog()?;
        println!("[vda] watchdog timeout {timeout} s (countdown {countdown})");
        start_watchdog_ping();
        Some(Arc::new(Mutex::new(Display {
            vda,
            anchor: (real.rect.right, real.rect.bottom),
            size: (w, h),
            is_on: false,
        })))
    };

    let result = mirror_until_quit(&real, area, display.clone());

    if let Some(d) = &display
        && let Err(e) = d.lock().unwrap().off()
    {
        eprintln!("[vda] remove failed: {e} (the driver watchdog removes it within its timeout)");
    }
    *CLEANED_UP.0.lock().unwrap() = true;
    CLEANED_UP.1.notify_all();
    println!("bye");
    result
}

/// Pings the driver every second on its own handle and thread, independent of UI and capture.
fn start_watchdog_ping() {
    thread::spawn(|| {
        let vda = match Vda::open() {
            Ok(v) => v,
            Err(e) => return eprintln!("[watchdog] cannot open a second handle: {e}"),
        };
        let mut fails = 0u32;
        loop {
            match vda.ping() {
                Ok(()) => fails = 0,
                Err(e) => {
                    fails += 1;
                    eprintln!("[watchdog] ping failed ({fails} in a row): {e}");
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

/// The window rect the presenter must cover: the Virtual monitor, or top-left of the real one in preview.
fn presenter_rect(display: &Option<Arc<Mutex<Display>>>, area: Area) -> Result<RECT> {
    Ok(match display {
        Some(d) => {
            let m = d.lock().unwrap().on()?;
            RECT {
                left: m.x,
                top: m.y,
                right: m.x + m.w as i32,
                bottom: m.y + m.h as i32,
            }
        }
        None => RECT {
            left: 0,
            top: 0,
            right: area.w as i32,
            bottom: area.h as i32,
        },
    })
}

fn place(hwnd: isize, r: RECT) {
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd as _),
            Some(HWND_TOPMOST),
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

fn hide(hwnd: isize) {
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd as _),
            None,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_HIDEWINDOW,
        );
    }
}

fn mirror_until_quit(real: &Real, area: Area, display: Option<Arc<Mutex<Display>>>) -> Result<()> {
    let rect = presenter_rect(&display, area)?;
    let gpu = mirror::create_device()?;
    let hwnd = create_window(rect)?;
    let mirror = Arc::new(Mutex::new(Mirror::new(&gpu, hwnd, area)?));
    // Adding the Virtual monitor changes the display topology, which invalidates the HMONITOR taken at startup.
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let mut capture = mirror::start_capture(&gpu, monitor, mirror.clone())?;
    println!("[border] look at the real monitor now: is a yellow capture border drawn around it?");

    unsafe {
        MAIN_TID.store(GetCurrentThreadId(), Ordering::SeqCst);
        // A parent can hand us "ignore Ctrl+C" by inheritance; take it back so Ctrl+C always cleans up.
        SetConsoleCtrlHandler(None, false)?;
        SetConsoleCtrlHandler(Some(on_console_event), true)?;
    }

    {
        let mirror = mirror.clone();
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(5));
                print_stats(&mirror);
            }
        });
    }
    {
        let (mirror, real) = (mirror.clone(), real.size());
        let hwnd = hwnd.0 as isize;
        let display = display.clone();
        thread::spawn(move || {
            println!("{HELP}");
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                command(line.trim(), &mirror, display.as_deref(), hwnd, real);
            }
            println!("[stdin] closed; quit with Ctrl+C");
        });
    }

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_RECAPTURE {
                // A monitor add/remove kills the WGC capture of the real screen; start a fresh one.
                let _ = capture.session.Close();
                let _ = capture.pool.Close();
                let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
                match mirror::start_capture(&gpu, monitor, mirror.clone()) {
                    Ok(c) => {
                        capture = c;
                        println!("[wgc] capture restarted");
                    }
                    Err(e) => println!("[wgc] capture restart failed: {e}"),
                }
                continue;
            }
            DispatchMessageW(&msg);
        }
    }

    println!("[quit] stopping capture");
    mirror.lock().unwrap().presenting = false;
    let _ = capture.session.Close();
    let _ = capture.pool.Close();
    print_stats(&mirror);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

fn print_stats(mirror: &Mutex<Mirror>) {
    let mut m = mirror.lock().unwrap();
    println!("{}", m.stats.report());
    if let Some(info) = &m.info {
        mirror::drain_debug_messages(info);
    }
}

fn command(
    line: &str,
    mirror: &Mutex<Mirror>,
    display: Option<&Mutex<Display>>,
    hwnd: isize,
    real: (u32, u32),
) {
    let parts: Vec<&str> = line.split_whitespace().collect();
    match parts.as_slice() {
        [] => {}
        ["q"] => unsafe {
            let _ = PostThreadMessageW(
                MAIN_TID.load(Ordering::SeqCst),
                WM_QUIT,
                WPARAM(0),
                LPARAM(0),
            );
        },
        ["s"] => print_stats(mirror),
        ["m", x, y] => {
            let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>()) else {
                return println!("bad move: {line}  ({HELP})");
            };
            let mut m = mirror.lock().unwrap();
            m.area.x = x;
            m.area.y = y;
            let (l, t, r, b) = m.area.clamp(real.0, real.1);
            println!(
                "[move] Area at ({x},{y}); copied box {}x{} at ({l},{t}){}",
                r - l,
                b - t,
                if (r - l, b - t) == (m.area.w as i32, m.area.h as i32) {
                    ""
                } else {
                    "  (clamped, rest is black)"
                }
            );
        }
        ["r", a, b] => match parse_size(a, b, real) {
            Err(e) => println!("bad resize: {e}"),
            Ok((w, h)) => resize(w, h, mirror, display, hwnd),
        },
        ["off"] => {
            mirror.lock().unwrap().presenting = false;
            hide(hwnd); // before the remove, or Windows moves the window onto the real monitor
            match display {
                Some(d) => {
                    if let Err(e) = d.lock().unwrap().off() {
                        println!("[off] remove failed: {e}");
                    }
                }
                None => println!("[off] presenter hidden (preview)"),
            }
        }
        ["on"] => {
            if mirror.lock().unwrap().presenting {
                return println!("[on] already on");
            }
            let area = mirror.lock().unwrap().area;
            let rect = match display {
                Some(d) => match d.lock().unwrap().on() {
                    Ok(m) => RECT {
                        left: m.x,
                        top: m.y,
                        right: m.x + m.w as i32,
                        bottom: m.y + m.h as i32,
                    },
                    Err(e) => return println!("[on] failed: {e}"),
                },
                None => RECT {
                    left: 0,
                    top: 0,
                    right: area.w as i32,
                    bottom: area.h as i32,
                },
            };
            place(hwnd, rect);
            mirror.lock().unwrap().presenting = true;
            if display.is_some() {
                recapture();
            }
            println!("[on] presenting");
        }
        _ => println!("unknown command: {line:?}\n{HELP}"),
    }
}

/// ResizeBuffers, then (driver, while Sharing) hide, remove, re-add at the new size, set mode +
/// position, move, show. With Sharing off only the size is stored; `on` adds at it.
/// Preview: ResizeBuffers + resize the window.
fn resize(w: u32, h: u32, mirror: &Mutex<Mirror>, display: Option<&Mutex<Display>>, hwnd: isize) {
    let t = Instant::now();
    let was_presenting = {
        let mut m = mirror.lock().unwrap();
        if let Err(e) = m.resize(w, h) {
            return println!("[resize] ResizeBuffers failed: {e}");
        }
        std::mem::replace(&mut m.presenting, false)
    };
    let t_swap = t.elapsed();
    let rect = match display {
        Some(d) => {
            let mut d = d.lock().unwrap();
            d.size = (w, h);
            if !was_presenting {
                return println!(
                    "[resize] Sharing is off: the next `on` adds the Virtual monitor at {w}x{h}"
                );
            }
            hide(hwnd); // before the remove, or Windows moves the window onto the real monitor
            if let Err(e) = d.off() {
                return println!("[resize] remove failed, Sharing is off now: {e}");
            }
            match d.on() {
                Ok(m) => RECT {
                    left: m.x,
                    top: m.y,
                    right: m.x + m.w as i32,
                    bottom: m.y + m.h as i32,
                },
                Err(e) => {
                    return println!(
                        "[resize] re-add failed, Sharing is off now (`on` retries): {e}"
                    );
                }
            }
        }
        None => RECT {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h as i32,
        },
    };
    if was_presenting {
        place(hwnd, rect);
        if display.is_some() {
            recapture();
        }
    }
    mirror.lock().unwrap().presenting = was_presenting;
    println!(
        "[resize] {w}x{h} done in {:.0} ms (swapchain {:.0} ms, monitor + window {:.0} ms)",
        t.elapsed().as_secs_f64() * 1e3,
        t_swap.as_secs_f64() * 1e3,
        (t.elapsed() - t_swap).as_secs_f64() * 1e3
    );
}

/// Ctrl+C, Ctrl+Break, console close, logoff, shutdown: ask the main thread to quit and wait for cleanup.
extern "system" fn on_console_event(kind: u32) -> BOOL {
    println!(
        "[console] event {kind}{}, shutting down",
        if kind == CTRL_C_EVENT {
            " (Ctrl+C)"
        } else {
            ""
        }
    );
    unsafe {
        let _ = PostThreadMessageW(
            MAIN_TID.load(Ordering::SeqCst),
            WM_QUIT,
            WPARAM(0),
            LPARAM(0),
        );
    }
    let done = CLEANED_UP.0.lock().unwrap();
    let _ = CLEANED_UP
        .1
        .wait_timeout_while(done, Duration::from_secs(4), |d| !*d);
    TRUE
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DPICHANGED => LRESULT(0), // keep our exact pixel size on any monitor
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn create_window(r: RECT) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?.into();
        let class = w!("cropcast-spike-presenter");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        });
        // No WDA_EXCLUDEFROMCAPTURE here: Discord has to see this window.
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("cropcast spike"),
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
        Ok(hwnd)
    }
}
