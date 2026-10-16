# Mirror pipeline: Area to Virtual monitor at 60 fps

Research for issue #4 (map: #1). Researched 2026-10-16 against Microsoft docs, the windows-rs docs and the source of OBS, Sunshine and the windows-capture crate. Vocabulary follows [GLOSSARY.md](../../GLOSSARY.md).

## Recommendation

```
Windows.Graphics.Capture (monitor item: the real screen, cursor on, border off, MinUpdateInterval ~16 ms)
  -> FrameArrived on a free-threaded frame pool (2 buffers, B8G8R8A8)
  -> ID3D11DeviceContext::CopySubresourceRegion(swapchain back buffer, box = Area)   [one GPU copy]
  -> IDXGISwapChain1::Present on a flip-model swapchain (FLIP_DISCARD, 2 buffers)
     owned by a borderless topmost no-activate window that exactly covers the Virtual monitor
  -> DWM composes the Virtual monitor -> IddCx driver -> Discord captures the Virtual monitor
```

- **Capture API: Windows.Graphics.Capture (WGC)**, not DXGI Desktop Duplication. It draws the cursor for us, survives mode/fullscreen switches without the `DXGI_ERROR_ACCESS_LOST` re-create dance, and honours `WDA_EXCLUDEFROMCAPTURE` the same way. Desktop Duplication stays as a fallback only.
- **Copy path: one GPU-to-GPU `CopySubresourceRegion`** from the WGC frame texture straight into the swapchain back buffer, with a `D3D11_BOX` equal to the Area in monitor coordinates. No CPU readback, no staging texture, no shader. Same D3D11 device for capture and present, so no cross-device sharing.
- **Present path: a window on the Virtual monitor with a flip-model swapchain.** No IddCx driver exposes a standard "push frames in" API, and writing our own (signed) driver for that is out of proportion; the window is cheap and normal.
- **Cursor: `IsCursorCaptureEnabled = true`.** DWM renders the cursor into the capture, exactly like a normal share. Nothing to draw ourselves.
- **Rust: plain `windows` (windows-rs), not the windows-capture crate.** About 150 lines.
- **Expected GPU cost: well under 1% of an RX 6800 XT.** See [Cost](#cost).

## This PC (measured 2026-10-16)

Queried with `QueryDisplayConfig` + `DisplayConfigGetDeviceInfo(DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO)`:

| Display | HDR supported | HDR (advanced color) enabled | Encoding | Bits/channel | Refresh |
|---|---|---|---|---|---|
| Samsung Odyssey G93SC (5120x1440) | yes | **no** | RGB | 10 | **240 Hz** |

GPU: AMD Radeon RX 6800 XT (single GPU, no iGPU, so no hybrid-GPU capture issues). Windows 11 build 26100 (24H2).

Consequences: the desktop is SDR, so capture in `B8G8R8A8UIntNormalized` and present `DXGI_FORMAT_B8G8R8A8_UNORM`; no tone mapping needed. The monitor runs at 240 Hz, so WGC must be throttled to 60 fps (240/60 = 4, an even divisor, so pacing is clean).

## Capture: WGC vs Desktop Duplication

| | Windows.Graphics.Capture | DXGI Desktop Duplication |
|---|---|---|
| Cursor | `IsCursorCaptureEnabled` puts it in the frame ([docs][wgc-cursor], Win10 2004+) | Separate pointer shape + position; the app must draw it when the hardware overlays it ([docs][dd]) |
| Mode change / fullscreen switch | Keeps running; frame size may change, handled with `Recreate` (OBS does this) | `AcquireNextFrame` returns `DXGI_ERROR_ACCESS_LOST` on desktop switch, mode change, or "other full-screen application"; must tear down and re-create ([docs][anf]) |
| Rate control | `MinUpdateInterval` (present on 26100; check with `ApiInformation::IsPropertyPresent`, as windows-capture and Sunshine do) | Poll `AcquireNextFrame` with a timeout |
| Crop | None; capture the monitor, crop on copy | None; same |
| Excludes `WDA_EXCLUDEFROMCAPTURE` windows | Yes | Yes |
| Capture indicator | Yellow border unless `IsBorderRequired = false` (see gotchas) | None |

Prior art agrees: OBS offers both for display capture and uses WGC with `CopySubresourceRegion` + a box for its client-area crop ([winrt-capture.cpp][obs]). Sunshine supports both; its WGC path uses a free-threaded pool of 2 frames, sets `IsBorderRequired(false)` and `MinUpdateInterval` (it sets 4 ms because without it "Screen capture may be capped to 60fps" — that default cap is fine for us, but set it explicitly) ([display_wgc.cpp][sun-wgc]).

## Keeping the Border out

`SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`: "The window is displayed only on a monitor. Everywhere else, the window does not appear at all", intended for recording controls ([docs][wda]). Constraints from the same page:
- `hwnd` must be a top-level window owned by our process.
- Only works while DWM composes the desktop (always true on Windows 11).
- Windows 10 2004+; older builds fall back to `WDA_MONITOR` (black box). Not a concern on 26100.
- It is not a security guarantee, but it is exactly the mechanism WGC and Desktop Duplication honour.

So the Border window gets `WDA_EXCLUDEFROMCAPTURE`; our WGC capture of the real screen then never contains it, and since the Virtual monitor only shows our copy, Discord never sees it. **The presenter window on the Virtual monitor must NOT get this flag**, or Discord sees nothing.

## Present: window + flip-model swapchain

- Window: `WS_POPUP`, `WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` (never steals focus from the game, no taskbar button), sized to the Virtual monitor's rect from `GetMonitorInfo`. Avoid `WS_EX_LAYERED`/GDI drawing on it (flip model cannot mix producers on one HWND, [docs][flip]).
- Swapchain: `CreateSwapChainForHwnd`, `DXGI_SWAP_EFFECT_FLIP_DISCARD`, 2 buffers, `B8G8R8A8_UNORM`, `DXGI_SCALING_NONE`, flag `FRAME_LATENCY_WAITABLE_OBJECT` with max latency 1 ([docs][flip]). On an Area resize: `ResizeBuffers` to the new Area size (the Virtual monitor mode changes with it; that is the driver ticket's job).
- Present on frame arrival with sync interval 0 (we are already paced by WGC; with flip-discard DWM shows the newest frame at the Virtual monitor's vblank). If nothing changed, WGC sends no frame and we present nothing; DWM keeps showing the last one.

## Cost

Area example 2381x1307 BGRA = 12.4 MB/frame. At 60 fps:
- Our copy: read 12.4 MB + write 12.4 MB = ~1.5 GB/s.
- DWM's copy into the WGC pool (full 5120x1440 = 29.5 MB, but only at the 60 fps we allow): ~1.8 GB/s write.
- DWM composing the Virtual monitor + Discord's own capture: roughly the same again.

Total ~5-6 GB/s against 512 GB/s of RX 6800 XT memory bandwidth: about 1%, CPU near zero (no readback). Even at 2560x1440 defaults the numbers barely move. Latency: one WGC hop (<= 1 frame) + one Virtual-monitor vblank (<= 16.7 ms) before Discord sees it; nothing is added to the game's own present path.

## Rust

- `windows` crate features: `Graphics_Capture`, `Graphics_DirectX_Direct3D11`, `Foundation`, `Foundation_Metadata`, `Win32_System_WinRT_Graphics_Capture` (`IGraphicsCaptureItemInterop::CreateForMonitor`), `Win32_System_WinRT_Direct3D11` (`CreateDirect3D11DeviceFromDXGIDevice`, `IDirect3DDxgiInterfaceAccess`), `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi(_Common)`, `Win32_UI_WindowsAndMessaging`. These are the same features windows-capture 2.0.1 uses (on windows 0.62.2) ([Cargo.toml][wc-toml]).
- windows-capture is a good reference but a poor fit: it creates its own D3D device and its own capture thread with a message loop, and its frame API is built around mapping to CPU buffers (`buffer`, `buffer_crop` via a staging copy) ([capture.rs][wc-cap], [frame.rs][wc-frame]). We need our device shared with the swapchain and a GPU-only path.
- Use `Direct3D11CaptureFramePool::CreateFreeThreaded` so `FrameArrived` fires on a worker thread without a DispatcherQueue (Sunshine does this).

## Gotchas

1. **240 Hz source.** Without `MinUpdateInterval`, WGC can deliver more frames than 60; set ~16 ms (Sunshine notes the default is capped to 60, but don't rely on it).
2. **`ContentSize` is unreliable**; read the real size from the texture's `GetDesc` (OBS comment). Call `framePool.Recreate` only when the *monitor* size changes, never on an Area change: Area changes only change the copy box and the swapchain size.
3. **Clamp the copy box** to the monitor texture; the source box of `CopySubresourceRegion` must lie inside the source texture (an out-of-range box is invalid and the copy does not happen as intended). Matters while the Area is dragged near the screen edges.
4. **Yellow capture border.** `IsBorderRequired = false` needs user consent via `GraphicsCaptureAccess.RequestAccessAsync(Borderless)`; if denied, setting false "succeeds" but is ignored. For packaged apps the `graphicsCaptureWithoutBorder` capability is required ([docs][border]). OBS calls `RequestAccessAsync(Borderless)` then sets false; Sunshine just sets false. Behaviour for our unpackaged exe: test.
5. **Presenter window must not take focus** (`WS_EX_NOACTIVATE`) or games lose focus/minimize.
6. **Cursor on the Virtual monitor.** Windows lays monitors out edge to edge, so the mouse (and new windows) can wander onto the Virtual monitor. Placement and possibly `ClipCursor` belong to the Virtual monitor ticket.
7. **Exclusive fullscreen** (true FSE, fullscreen optimizations off) can change what DWM composes; Desktop Duplication explicitly loses access there. WGC should keep running but may show black or stall during the switch: test.
8. **GPU contention under a GPU-bound game.** Sunshine raises its process to realtime GPU scheduling priority (`D3DKMTSetProcessSchedulingPriorityClass`, needs admin) and `SetGPUThreadPriority(7)` to stop capture stutter ([display_base.cpp][sun-base]). Try without first; our work is one small copy.
9. **HDR later.** If HDR is turned on, BGRA8 capture is clipped; OBS switches to `R16G16B16A16_FLOAT` capture for HDR monitors. The IddCx Virtual monitor is SDR unless the driver reports `IDDCX_ADAPTER_FLAGS_CAN_PROCESS_FP16` (IddCx 1.10). Simplest: keep BGRA8 and accept clipping, or capture FP16 and tone-map in a shader (adds a pass). Not needed today (HDR off).
10. **Protected (DRM) content** shows black in both APIs; Sunshine logs it. Expected, not a bug.

## Only a hands-on test can settle

- Whether WGC capture of the real screen changes a game's present mode (PresentMon: does a borderless game stay "Hardware: Independent Flip" while Sharing?) and whether it costs the game any frames at 240 Hz.
- Whether `IsBorderRequired = false` takes effect for an unpackaged exe without a prompt.
- Whether a true exclusive-fullscreen game is captured or goes black.
- End-to-end latency real screen -> Discord preview, and whether DWM composes or direct-flips our swapchain on the IddCx monitor.
- Discord's own behaviour when the Virtual monitor resizes mid-share (likely a separate ticket).

## Rejected alternatives

- **Desktop Duplication**: more code (cursor drawing, ACCESS_LOST recovery), no benefit on a single-GPU desktop.
- **Magnification API** (`MagSetWindowSource`): legacy, no control over pacing or format.
- **DWM thumbnails**: only for top-level windows, not a screen region.
- **Custom IddCx driver that takes frames directly**: would save one DWM composition, but needs a signed kernel-adjacent driver; not worth it at ~1% GPU.

[wgc-cursor]: https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.iscursorcaptureenabled
[border]: https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired
[dd]: https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
[anf]: https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe
[wda]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
[flip]: https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model
[obs]: https://github.com/obsproject/obs-studio/blob/master/libobs-winrt/winrt-capture.cpp
[sun-wgc]: https://github.com/LizardByte/Sunshine/blob/master/src/platform/windows/display_wgc.cpp
[sun-base]: https://github.com/LizardByte/Sunshine/blob/master/src/platform/windows/display_base.cpp
[wc-toml]: https://github.com/NiiightmareXD/windows-capture/blob/main/Cargo.toml
[wc-cap]: https://github.com/NiiightmareXD/windows-capture/blob/main/src/capture.rs
[wc-frame]: https://github.com/NiiightmareXD/windows-capture/blob/main/src/frame.rs
