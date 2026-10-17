//! WGC capture of the real monitor -> one CopySubresourceRegion of the Area -> flip-model swapchain.

use std::sync::{Arc, Mutex};

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
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{Interface, Result, factory};

/// The Area, in real-monitor pixels (monitor top-left = 0,0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Area {
    /// The default Area: 2560×1440 centered, shrunk to fit the monitor and kept even.
    pub fn default_for(mw: u32, mh: u32) -> Area {
        let (w, h) = (2560.min(mw) & !1, 1440.min(mh) & !1);
        Area { x: (mw - w) as i32 / 2, y: (mh - h) as i32 / 2, w, h }
    }

    /// The part of the Area inside a `tw`×`th` source, as (left, top, right, bottom); may be empty.
    fn clamp(&self, tw: u32, th: u32) -> (i32, i32, i32, i32) {
        let (tw, th) = (tw as i32, th as i32);
        (
            self.x.clamp(0, tw),
            self.y.clamp(0, th),
            (self.x + self.w as i32).clamp(0, tw),
            (self.y + self.h as i32).clamp(0, th),
        )
    }
}

pub struct Gpu {
    pub device: ID3D11Device,
    ctx: ID3D11DeviceContext,
}

/// Hardware device on the default adapter (the one driving the primary monitor).
pub fn create_device() -> Result<Gpu> {
    let (mut device, mut ctx) = (None, None);
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut ctx),
        )?;
        let (device, ctx): (ID3D11Device, ID3D11DeviceContext) = (device.unwrap(), ctx.unwrap());
        // WGC's thread-pool callbacks and the UI thread share this device.
        let _ = device.cast::<ID3D11Multithread>()?.SetMultithreadProtected(true);
        Ok(Gpu { device, ctx })
    }
}

pub struct Mirror {
    ctx: ID3D11DeviceContext,
    swap: IDXGISwapChain2,
    waitable: HANDLE,
    pub area: Area,
    /// Cleared before the capture stops, so a frame already in flight doesn't present.
    pub presenting: bool,
}

// ponytail: every use goes through Arc<Mutex<Mirror>> and the device is multithread-protected.
unsafe impl Send for Mirror {}

impl Mirror {
    pub fn new(gpu: &Gpu, hwnd: HWND, area: Area) -> Result<Mirror> {
        unsafe {
            let factory: IDXGIFactory2 = gpu.device.cast::<IDXGIDevice>()?.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: area.w,
                Height: area.h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_NONE,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
                ..Default::default()
            };
            let swap: IDXGISwapChain2 = factory.CreateSwapChainForHwnd(&gpu.device, hwnd, &desc, None, None)?.cast()?;
            swap.SetMaximumFrameLatency(1)?;
            let waitable = swap.GetFrameLatencyWaitableObject();
            Ok(Mirror { ctx: gpu.ctx.clone(), swap, waitable, area, presenting: true })
        }
    }

    /// New swapchain size for a resized Area. No back buffer is held between frames.
    pub fn resize(&mut self, area: Area) -> Result<()> {
        unsafe {
            let flags = DXGI_SWAP_CHAIN_FLAG(DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0);
            self.swap.ResizeBuffers(2, area.w, area.h, DXGI_FORMAT_B8G8R8A8_UNORM, flags)?;
        }
        self.area = area;
        Ok(())
    }

    /// One frame: wait for the swapchain, copy the Area box, black out any part off the source.
    fn present(&mut self, src: &ID3D11Texture2D) -> Result<()> {
        if !self.presenting {
            return Ok(());
        }
        unsafe {
            WaitForSingleObjectEx(self.waitable, 100, true);
            let mut sd = D3D11_TEXTURE2D_DESC::default();
            src.GetDesc(&mut sd);
            let a = self.area;
            let (l, t, r, b) = a.clamp(sd.Width, sd.Height);
            let back: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            if (l, t, r - l, b - t) != (a.x, a.y, a.w as i32, a.h as i32) {
                let mut rtv = None;
                self.ctx.GetDevice()?.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
                self.ctx.ClearRenderTargetView(rtv.as_ref().unwrap(), &[0.0, 0.0, 0.0, 1.0]);
            }
            if r > l && b > t {
                let bx = D3D11_BOX { left: l as u32, top: t as u32, front: 0, right: r as u32, bottom: b as u32, back: 1 };
                self.ctx.CopySubresourceRegion(&back, 0, (l - a.x) as u32, (t - a.y) as u32, 0, src, 0, Some(&bx));
            }
            drop(back);
            self.swap.Present(0, DXGI_PRESENT(0)).ok()
        }
    }
}

/// A running WGC capture. Dropping it stops the capture.
pub struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// Starts WGC on `monitor` (cursor on, no yellow border, ~60 fps) and feeds every frame to `mirror`.
pub fn start_capture(gpu: &Gpu, monitor: HMONITOR, mirror: Arc<Mutex<Mirror>>) -> Result<Capture> {
    // Asking first is what lets SetIsBorderRequired(false) work without a prompt (Allowed on Win11).
    let _ = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless).and_then(|op| op.join());
    unsafe {
        let item: GraphicsCaptureItem =
            factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?.CreateForMonitor(monitor)?;
        let d3d: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&gpu.device.cast::<IDXGIDevice>()?)?.cast()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            item.Size()?,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        session.SetIsCursorCaptureEnabled(true)?;
        let _ = session.SetIsBorderRequired(false);
        // The real monitor runs at 240 Hz; Discord tops out at 60 fps.
        session.SetMinUpdateInterval(TimeSpan { Duration: 160_000 })?; // 16 ms in 100 ns ticks
        pool.FrameArrived(&TypedEventHandler::new(move |pool: windows::core::Ref<Direct3D11CaptureFramePool>, _| {
            let Some(pool) = pool.as_ref() else { return Ok(()) };
            let tex: ID3D11Texture2D =
                pool.TryGetNextFrame()?.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()?;
            // ponytail: a failed frame is skipped; the next one ~16 ms later tries again.
            let _ = mirror.lock().unwrap().present(&tex);
            Ok(())
        }))?;
        session.StartCapture()?;
        Ok(Capture { pool, session })
    }
}

#[cfg(test)]
mod tests {
    use super::Area;

    #[test]
    fn default_area_is_centered_even_and_fits() {
        assert_eq!(Area::default_for(5120, 1440), Area { x: 1280, y: 0, w: 2560, h: 1440 });
        assert_eq!(Area::default_for(1920, 1080), Area { x: 0, y: 0, w: 1920, h: 1080 });
        assert_eq!(Area::default_for(1366, 767), Area { x: 0, y: 0, w: 1366, h: 766 });
    }

    #[test]
    fn clamp_cuts_the_area_to_the_source() {
        let a = Area { x: -10, y: 5, w: 100, h: 100 };
        assert_eq!(a.clamp(50, 50), (0, 5, 50, 50));
        assert_eq!(Area { x: 60, ..a }.clamp(50, 50), (50, 5, 50, 50)); // fully off: empty
    }
}
