# 0001 — Mirror the Area through a window on a Virtual monitor

Status: accepted, 2026-10-16 ([Lock the capture route](https://github.com/lumi-fiona/cropcast/issues/5))

## Context

Discord can share a whole screen (with system audio) or a single window, never part of a screen. Lumi's screen is 5120×1440; she wants viewers to see only the Area, pixel for pixel.

## Decision

When turned on, cropcast adds a Virtual monitor exactly the size of the Area and paints the Area into a window on it at 60 fps. Discord shares that screen.

- **Driver:** stock SudoVDA, built and signed in CI. Monitors are added and removed by IOCTL at any size; a 3 s watchdog ping keeps them alive.
- **Capture:** Windows.Graphics.Capture of the real monitor (cursor on, capture border off, ~16 ms interval, BGRA8).
- **Present:** one GPU copy of the Area per frame into a flip-model swapchain on a borderless, topmost, no-activate window covering the Virtual monitor.
- **Edit mode:** a move only shifts the copy box and is live. A resize removes and re-adds the Virtual monitor at the new size, once, when Edit mode ends.
- **Sizes:** the Area is always an even number of pixels wide and tall (video encoders need it), so nothing is padded.
- **Placement:** the Virtual monitor touches the real one only at its bottom-right corner, so the cursor and new windows rarely wander onto it.

## Rejected

- **Feeding frames into the driver directly:** Discord captures the screen from DWM, before the driver sees any frame, so viewers would never see them.
- **DXGI Desktop Duplication:** it drops capture on every mode change and needs the cursor drawn by hand. Supporting older Windows is out of scope for v1.
- **Forking SudoVDA for live resize up front:** only if the spike shows that remove + re-add ends the Discord share.
- **Confining the cursor with `ClipCursor`:** it fights games and other apps.

## Consequences

- A resize briefly glitches the stream; Discord rebuilds its capture.
- Anything else that lands on the Virtual monitor (a stray window, the taskbar, the cursor) is seen by viewers.
- Accepted v1 limits: a sharp stream needs Nitro with the Source preset (or an Area that fits 1440p); exclusive-fullscreen games may mirror black; SDR only; no admin-only realtime GPU priority.
