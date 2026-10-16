//! WGC capture of the real monitor -> one CopySubresourceRegion of the Area -> flip-model swapchain.

use std::mem::size_of;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::*;
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Threading::WaitForSingleObjectEx;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{Interface, Result, factory};

/// The Area, in real-monitor pixels (monitor top-left = 0,0).
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Area {
    /// The part of the Area inside a `tw`×`th` source, as (left, top, right, bottom); may be empty.
    pub fn clamp(&self, tw: u32, th: u32) -> (i32, i32, i32, i32) {
        let (tw, th) = (tw as i32, th as i32);
        (
            self.x.clamp(0, tw),
            self.y.clamp(0, th),
            (self.x + self.w as i32).clamp(0, tw),
            (self.y + self.h as i32).clamp(0, th),
        )
    }
}

#[derive(Default)]
pub struct Stats {
    since: Option<Instant>,
    captured: u32,
    presented: u32,
    last_present: Option<Instant>,
    max_gap: Duration,
    late: u32,
    max_wait: Duration,
    wait_timeouts: u32,
    clamped: u32,
    errors: u32,
    gpu_sum_us: f64,
    gpu_n: u32,
    gpu_max_us: f64,
}

impl Stats {
    /// One line of rates since the last report, then resets the window.
    pub fn report(&mut self) -> String {
        let now = Instant::now();
        let secs = self
            .since
            .map_or(0.0, |s| (now - s).as_secs_f64())
            .max(1e-3);
        let gpu = if self.gpu_n > 0 {
            format!(
                "gpu-copy avg {:.3} ms max {:.3} ms",
                self.gpu_sum_us / self.gpu_n as f64 / 1e3,
                self.gpu_max_us / 1e3
            )
        } else {
            "gpu-copy n/a".into()
        };
        let line = format!(
            "[stats {secs:.1}s] captured {:.1}/s  presented {:.1}/s  max-gap {:.1} ms  late(>25ms) {}  max-wait {:.1} ms  wait-timeouts {}  clamped {}  errors {}  {gpu}",
            self.captured as f64 / secs,
            self.presented as f64 / secs,
            self.max_gap.as_secs_f64() * 1e3,
            self.late,
            self.max_wait.as_secs_f64() * 1e3,
            self.wait_timeouts,
            self.clamped,
            self.errors,
        );
        *self = Stats {
            since: Some(now),
            last_present: self.last_present,
            ..Default::default()
        };
        line
    }
}

pub struct Gpu {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub info: Option<ID3D11InfoQueue>,
}

/// Hardware device on the default adapter (the one driving the primary monitor).
/// Debug builds try the D3D11 debug layer, which needs the "Graphics Tools" optional feature.
pub fn create_device() -> Result<Gpu> {
    let base = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    let create = |flags| {
        let (mut device, mut ctx) = (None, None);
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
        }
        .map(|_| (device.unwrap(), ctx.unwrap()))
    };
    let (device, ctx) = if cfg!(debug_assertions) {
        match create(base | D3D11_CREATE_DEVICE_DEBUG) {
            Ok(d) => {
                println!("[d3d] debug layer ON");
                d
            }
            Err(e) => {
                println!(
                    "[d3d] debug layer not available ({e}); install the Windows 'Graphics Tools' optional feature to get it"
                );
                create(base)?
            }
        }
    } else {
        create(base)?
    };
    // WGC and our frame thread share this device; the command thread resizes the swapchain.
    let _ = unsafe {
        device
            .cast::<ID3D11Multithread>()?
            .SetMultithreadProtected(true)
    };
    let info = device.cast::<ID3D11InfoQueue>().ok();
    Ok(Gpu { device, ctx, info })
}

/// Prints and clears D3D11 debug-layer messages (debug builds with the layer installed only).
pub fn drain_debug_messages(info: &ID3D11InfoQueue) {
    unsafe {
        for i in 0..info.GetNumStoredMessages() {
            let mut len = 0usize;
            let _ = info.GetMessage(i, None, &mut len);
            let mut buf = vec![0u64; len.div_ceil(8)];
            let msg = buf.as_mut_ptr() as *mut D3D11_MESSAGE;
            if info.GetMessage(i, Some(msg), &mut len).is_ok() {
                let m = &*msg;
                let text = std::slice::from_raw_parts(
                    m.pDescription,
                    m.DescriptionByteLength.saturating_sub(1),
                );
                println!(
                    "[d3d-debug] severity {} id {}: {}",
                    m.Severity.0,
                    m.ID.0,
                    String::from_utf8_lossy(text)
                );
            }
        }
        info.ClearStoredMessages();
    }
}

struct Timing {
    disjoint: ID3D11Query,
    start: ID3D11Query,
    end: ID3D11Query,
    pending: bool,
}

pub struct Mirror {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    swap: IDXGISwapChain2,
    waitable: HANDLE,
    timing: Timing,
    pub area: Area,
    pub presenting: bool,
    pub stats: Stats,
    pub info: Option<ID3D11InfoQueue>,
}

// ponytail: every use goes through Arc<Mutex<Mirror>> and the device is multithread-protected.
unsafe impl Send for Mirror {}

const SWAP_FLAGS: DXGI_SWAP_CHAIN_FLAG = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT;

impl Mirror {
    pub fn new(gpu: &Gpu, hwnd: HWND, area: Area) -> Result<Mirror> {
        unsafe {
            let factory: IDXGIFactory2 = gpu
                .device
                .cast::<IDXGIDevice>()?
                .GetAdapter()?
                .GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: area.w,
                Height: area.h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_NONE,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: SWAP_FLAGS.0 as u32,
                ..Default::default()
            };
            let swap: IDXGISwapChain2 = factory
                .CreateSwapChainForHwnd(&gpu.device, hwnd, &desc, None, None)?
                .cast()?;
            swap.SetMaximumFrameLatency(1)?;
            let waitable = swap.GetFrameLatencyWaitableObject();

            let query = |q| {
                let mut out = None;
                gpu.device
                    .CreateQuery(
                        &D3D11_QUERY_DESC {
                            Query: q,
                            MiscFlags: 0,
                        },
                        Some(&mut out),
                    )
                    .map(|_| out.unwrap())
            };
            let timing = Timing {
                disjoint: query(D3D11_QUERY_TIMESTAMP_DISJOINT)?,
                start: query(D3D11_QUERY_TIMESTAMP)?,
                end: query(D3D11_QUERY_TIMESTAMP)?,
                pending: false,
            };
            Ok(Mirror {
                device: gpu.device.clone(),
                ctx: gpu.ctx.clone(),
                swap,
                waitable,
                timing,
                area,
                presenting: true,
                stats: Stats {
                    since: Some(Instant::now()),
                    ..Default::default()
                },
                info: gpu.info.clone(),
            })
        }
    }

    /// ResizeBuffers to a new Area size. No back-buffer references are held outside `present`.
    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        unsafe {
            self.swap
                .ResizeBuffers(2, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, SWAP_FLAGS)?
        };
        self.area.w = w;
        self.area.h = h;
        Ok(())
    }

    /// Reads last frame's GPU copy time if the GPU is done with it; never blocks.
    fn poll_timing(&mut self) {
        let t = &mut self.timing;
        if !t.pending {
            return;
        }
        unsafe {
            // GetData returns S_FALSE (still Ok) while not ready and leaves the buffer alone,
            // so a zero Frequency means "not yet".
            let mut dj = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT::default();
            let flags = D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32;
            let _ = self.ctx.GetData(
                &t.disjoint,
                Some(&mut dj as *mut _ as _),
                size_of::<D3D11_QUERY_DATA_TIMESTAMP_DISJOINT>() as u32,
                flags,
            );
            if dj.Frequency == 0 {
                return;
            }
            t.pending = false;
            let (mut a, mut b) = (0u64, 0u64);
            let _ = self
                .ctx
                .GetData(&t.start, Some(&mut a as *mut _ as _), 8, flags);
            let _ = self
                .ctx
                .GetData(&t.end, Some(&mut b as *mut _ as _), 8, flags);
            if !dj.Disjoint.as_bool() && b > a && a > 0 {
                let us = (b - a) as f64 * 1e6 / dj.Frequency as f64;
                self.stats.gpu_sum_us += us;
                self.stats.gpu_n += 1;
                self.stats.gpu_max_us = self.stats.gpu_max_us.max(us);
            }
        }
    }

    /// One frame: wait for the swapchain, copy the clamped Area box, present with sync interval 0.
    pub fn present(&mut self, src: &ID3D11Texture2D) -> Result<()> {
        if !self.presenting {
            return Ok(());
        }
        self.poll_timing();
        unsafe {
            let t = Instant::now();
            if WaitForSingleObjectEx(self.waitable, 100, true) == WAIT_TIMEOUT {
                self.stats.wait_timeouts += 1;
            }
            self.stats.max_wait = self.stats.max_wait.max(t.elapsed());

            let mut sd = D3D11_TEXTURE2D_DESC::default();
            src.GetDesc(&mut sd);
            let a = self.area;
            let (l, t, r, b) = a.clamp(sd.Width, sd.Height);
            let back: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            if (l, t, r - l, b - t) != (a.x, a.y, a.w as i32, a.h as i32) {
                // Part of the Area is off the source: black out what the copy won't cover.
                self.stats.clamped += 1;
                let mut rtv = None;
                self.device
                    .CreateRenderTargetView(&back, None, Some(&mut rtv))?;
                self.ctx
                    .ClearRenderTargetView(rtv.as_ref().unwrap(), &[0.0, 0.0, 0.0, 1.0]);
            }
            if r > l && b > t {
                let time = !self.timing.pending;
                if time {
                    self.ctx.Begin(&self.timing.disjoint);
                    self.ctx.End(&self.timing.start);
                }
                let bx = D3D11_BOX {
                    left: l as u32,
                    top: t as u32,
                    front: 0,
                    right: r as u32,
                    bottom: b as u32,
                    back: 1,
                };
                self.ctx.CopySubresourceRegion(
                    &back,
                    0,
                    (l - a.x) as u32,
                    (t - a.y) as u32,
                    0,
                    src,
                    0,
                    Some(&bx),
                );
                if time {
                    self.ctx.End(&self.timing.end);
                    self.ctx.End(&self.timing.disjoint);
                    self.timing.pending = true;
                }
            }
            drop(back);
            self.swap.Present(0, DXGI_PRESENT(0)).ok()?;
        }
        let now = Instant::now();
        if let Some(last) = self.stats.last_present {
            self.stats.max_gap = self.stats.max_gap.max(now - last);
            if now - last > Duration::from_millis(25) {
                self.stats.late += 1;
            }
        }
        self.stats.last_present = Some(now);
        self.stats.presented += 1;
        Ok(())
    }
}

pub struct Capture {
    pub pool: Direct3D11CaptureFramePool,
    pub session: GraphicsCaptureSession,
}

/// Starts WGC on `monitor` and wires every frame into `mirror.present`.
pub fn start_capture(gpu: &Gpu, monitor: HMONITOR, mirror: Arc<Mutex<Mirror>>) -> Result<Capture> {
    if !GraphicsCaptureSession::IsSupported()? {
        return Err(windows::core::Error::new(
            E_FAIL,
            "Windows.Graphics.Capture is not supported here",
        ));
    }
    // Ask for borderless capture before turning the border off; log what Windows says.
    match GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
        .and_then(|op| op.join())
    {
        Ok(s) => {
            let name = [
                "DeniedBySystem",
                "NotDeclaredByApp",
                "DeniedByUser",
                "UserPromptRequired",
                "Allowed",
            ];
            println!(
                "[wgc] RequestAccessAsync(Borderless) -> {}",
                name.get(s.0 as usize).unwrap_or(&"unknown")
            );
        }
        Err(e) => println!("[wgc] RequestAccessAsync(Borderless) failed: {e}"),
    }
    unsafe {
        let item: GraphicsCaptureItem =
            factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?
                .CreateForMonitor(monitor)?;
        let d3d: IDirect3DDevice =
            CreateDirect3D11DeviceFromDXGIDevice(&gpu.device.cast::<IDXGIDevice>()?)?.cast()?;
        let size = item.Size()?;
        println!("[wgc] capture item {}x{}", size.Width, size.Height);
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        session.SetIsCursorCaptureEnabled(true)?;
        let set = session.SetIsBorderRequired(false);
        println!(
            "[wgc] SetIsBorderRequired(false) -> {set:?}; IsBorderRequired now reads {:?}",
            session.IsBorderRequired()
        );
        // Dial: SPIKE_INTERVAL_MS (default 16) to tune pacing against the 240 Hz source.
        let ms: f64 = std::env::var("SPIKE_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(16.0);
        session.SetMinUpdateInterval(TimeSpan {
            Duration: (ms * 1e4) as i64,
        })?; // 100 ns ticks
        println!(
            "[wgc] MinUpdateInterval reads {:?}",
            session.MinUpdateInterval().map(|t| t.Duration as f64 / 1e4)
        );

        pool.FrameArrived(&TypedEventHandler::new(
            move |pool: windows::core::Ref<Direct3D11CaptureFramePool>, _| {
                let Some(pool) = pool.as_ref() else {
                    return Ok(());
                };
                let frame = pool.TryGetNextFrame()?;
                let tex: ID3D11Texture2D = frame
                    .Surface()?
                    .cast::<IDirect3DDxgiInterfaceAccess>()?
                    .GetInterface()?;
                let mut m = mirror.lock().unwrap();
                m.stats.captured += 1;
                if let Err(e) = m.present(&tex) {
                    m.stats.errors += 1;
                    if m.stats.errors <= 5 {
                        eprintln!("[present] {e}");
                    }
                }
                Ok(())
            },
        ))?;
        session.StartCapture()?;
        Ok(Capture { pool, session })
    }
}
