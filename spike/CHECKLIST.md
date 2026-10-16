# Spike #6 checklist: Virtual monitor shared in Discord

Hands-on checks for [#6](https://github.com/lumi-fiona/cropcast/issues/6), run by Lumi with one Discord viewer on another machine. Fill in every **Result** cell. Terms as in [GLOSSARY.md](../GLOSSARY.md).

## How to run

1. **Build** (any terminal, not elevated): `cd spike` then `cargo build --release`.
2. **Get the driver package** (not elevated): `powershell -ExecutionPolicy Bypass -File driver\fetch.ps1`. It downloads Apollo 0.4.6, checks its SHA256, extracts SudoVDA plus `nefconc.exe` into `driver\pkg\` and prints every file's hash. It needs 7-Zip.
3. **Install the driver** (elevated PowerShell): `powershell -ExecutionPolicy Bypass -File driver\install.ps1`. This plants the `CN=sudovda@su.mk` certificate in Root and TrustedPublisher. It ends with "SudoVDA is up", or says why not.
4. **Run the spike** (normal terminal): `target\release\cropcast-spike.exe [W H]`. The default Area is 2560×1440, centred on the real monitor. It prints the driver protocol version, the watchdog, the Virtual monitor's GDI name, the mode Windows actually applied (`OK` or `MISMATCH`), the WGC border/consent results, and then stats every 5 s.
   - Commands (type, then Enter): `m X Y` moves the Area (real-monitor pixels, top-left = 0,0). `r W H` resizes it and prints how long that took. `off` / `on` toggles Sharing. `s` prints stats now. `q` quits.
   - Quitting with `q`, Ctrl+C or closing the window removes the Virtual monitor. If the process is killed, the driver's watchdog removes it within ~3 s.
   - `--preview` runs without the driver: the presenter sits at the real monitor's top-left and mirrors a 640×360 Area.
5. **Test pattern:** `spike\testpattern.png` (2560×1440) is a 1-px black/white checkerboard with a 1-px red frame, a 1-px green centre crosshair and blue ticks every 100 px. Open it at **100 %** zoom so it fills the Area. Any scaling on the viewer's side shows up as moiré or grey mush.
6. **Undo everything:** see [Undo](#undo).

Discord setup for the whole list: Settings > Advanced > **Developer Mode** on. The viewer is on a second machine or account and joins the same voice channel. Share **Screen** (not Window) and pick the Virtual monitor.

## Driver and Virtual monitor

| # | Check | How | Result |
|---|---|---|---|
| D1 | Secure Boot state | `install.ps1` prints it. Also visible in `msinfo32` > Secure Boot State. | |
| D2 | Driver installs and loads with Secure Boot on, no test-signing | `install.ps1` ends with "SudoVDA is up". Device Manager > Display adapters lists SudoVDA with no warning icon. | |
| D3 | Protocol version | The spike's first `[vda] protocol version` line. | |
| D4 | Watchdog timeout | `[vda] watchdog timeout N s` | |
| D5 | Exact even non-standard size | `cropcast-spike 2382 1308`: the `[vda] mode set` line says `OK` with 2382x1308@60. Settings > Display shows 2382 × 1308. | |
| D6 | Exact odd size | `cropcast-spike 2381 1307`: `OK` with 2381x1307@60? | |
| D7 | Add time | ms in the `[vda] added`, `GDI name` and `mode set` lines | add ___ ms, name ___ ms, mode ___ ms |
| D8 | Watchdog cleanup | While running, kill the spike: `taskkill /F /IM cropcast-spike.exe`. Does the Virtual monitor disappear within ~3 s? | |
| D9 | Non-admin control | Steps D5/D6 ran from a non-elevated terminal? | |

## Placement (corner-only contact)

The spike asks for the Virtual monitor's top-left at the real monitor's bottom-right (5120, 1440).

| # | Check | How | Result |
|---|---|---|---|
| P1 | Windows kept the corner position | The `mode set` line: `at (5120,1440)` and `OK`, or did Windows snap it to an edge (`MISMATCH`, other coordinates)? Also Settings > Display layout. | |
| P2 | Cursor stays off the Virtual monitor | Move the mouse hard into the bottom-right corner, along the bottom edge and along the right edge, and fling it diagonally. Does it ever cross? | |
| P3 | Cursor in games | In a borderless game, flick the mouse towards bottom-right. Does the cursor or the camera escape? | |
| P4 | New windows | Open Notepad, Explorer and a browser while sharing. Do any open on the Virtual monitor (the viewer would see them)? | |
| P5 | Taskbar | Is there a taskbar on the Virtual monitor (Settings > Personalization > Taskbar > "Show my taskbar on all displays")? Does the viewer see it? | |
| P6 | Win+Shift+Arrow | Can a window be pushed onto the Virtual monitor with Win+Shift+Right? (Expected yes; record it.) | |

## Discord: picker, capture method, picture

| # | Check | How | Result |
|---|---|---|---|
| S1 | Virtual monitor in the Screen picker | Go Live / Share Screen > **Screens** tab. Is there an extra screen showing the Area's content? | |
| S2 | Capture method Discord uses | While streaming, open the Voice & Video **debug panel** that Developer Mode adds and go to its *Screen Share* tab. Which method's frame counter is rising (WGC / DXGI Desktop Duplication / other)? | |
| S3 | Stream resolution at Source | Nitro, quality **Source**, 60 fps. The viewer's stream debug info, or your Debug panel: what resolution is sent? | sent ___ × ___ |
| S4 | Pixel for pixel, even size | Area 2382×1308 with the test pattern. The viewer zooms the stream to 100 % (fullscreen + "actual size", or a screenshot inspected at 100 %). Is the checkerboard crisp, with no moiré or grey? Is the red frame 1 px on all four sides? | |
| S5 | Pixel for pixel, odd size | The same at 2381×1307. Is anything cropped or padded (red frame missing on one side, extra column or row)? | |
| S6 | 1440p preset | Quality 1440p, Area 2560×1440, then 2382×1308. Scaled or not? | |
| S7 | Text readability | Put small UI text (e.g. a code editor at 12 px) in the Area. Is it readable for the viewer? | |
| S8 | Cursor visible | The cursor moves inside the Area (a desktop app, not a game that hides it). Does the viewer see it? | |
| S9 | Stream frame rate | Something moving in the Area. The spike's stats show `captured ~60/s presented ~60/s`; the Debug panel shows ___ fps sent. | spike ___/s, Discord ___ fps |

## Live changes

| # | Check | How | Result |
|---|---|---|---|
| L1 | Live move is smooth | Type `m 0 0`, `m 2560 0`, `m 1280 0` while the viewer watches. Is there an instant jump with no freeze or black frame? | |
| L2 | Resize: share survives? | `r 1920 1080`, then `r 2382 1308`, then `r 2381 1307`. For each: does the share **survive** (continues at the new size), **freeze**, or **end** (viewer drops; you have to re-pick the screen)? | |
| L3 | Resize glitch length | Viewer's view: black/frozen for how long (stopwatch, or a screen recording on the viewer side)? Also note the spike's `[resize] ... done in N ms`. | spike ___ ms, viewer glitch ___ s |
| L4 | Stream size after resize | Does Discord's sent resolution follow the new size (S3 method)? | |
| L5 | 10 resizes in a row | Alternate `r 2382 1308` / `r 2560 1440` ten times, ~3 s apart. Does it still survive at the end? | |
| L6 | Sharing off → on | `off`, wait 5 s, `on`. Does the Discord share survive, or does it end or need re-picking? | |
| L7 | Off → on, long gap | `off`, wait 60 s, `on`. Same question. | |

## Capture border and consent

| # | Check | How | Result |
|---|---|---|---|
| B1 | Consent prompt | Did Windows show any prompt when the spike started? The spike logs `RequestAccessAsync(Borderless) -> ...` (preview on this PC: `Allowed`, no prompt). | |
| B2 | Yellow border on the real monitor | Look at the edges of the 5120×1440 screen while the spike runs. Is there a yellow outline? (The spike set `IsBorderRequired=false`, and it reads back `false`.) | |
| B3 | Yellow border in the stream | Does the viewer see any yellow edge? | |

## Audio

| # | Check | How | Result |
|---|---|---|---|
| A1 | Audio router inventory | Look at the tray and at Settings > System > Sound > Output (or `mmsys.cpl`). Are any of Sonar, Voicemeeter, FxSound, Nahimic or VB-Cable installed or used as the default output? Which device is Discord's output (Discord > Voice & Video > Output Device)? | |
| A2 | System audio present | Share with **"Also share system audio"** on, and play music. Does the viewer hear it? | |
| A3 | Echo test | **Speakers, not headphones.** The viewer talks. Do they hear themselves come back through your stream? | |
| A4 | Echo with router bypassed (only if A3 echoes and A1 found a router) | Set Discord's Output Device straight to the real speakers or headset and repeat A3. | |

## Games

Use one borderless (windowed-fullscreen) game, plus one exclusive-fullscreen game if available. For the game frametime check, use a repeatable scene (standing still in the same spot, or a benchmark).

**PresentMon steps** (the fullest data):

1. Download `PresentMon-2.x-x64.exe` from https://github.com/GameTechDev/PresentMon/releases. Run it from an **elevated** terminal (ETW needs admin or the Performance Log Users group).
2. Spike **off** (not running): start the game, stand in the scene, then run
   `PresentMon-2.x-x64.exe --process_name <game>.exe --output_file off.csv --timed 60 --terminate_after_timed`
3. Spike **on** (`cropcast-spike.exe`, sharing in Discord), same scene: the same command with `--output_file on.csv`.
4. Summarise each CSV (PowerShell; PresentMon 2.x writes `FrameTime`, 1.x writes `MsBetweenPresents`):
   ```powershell
   $r = Import-Csv on.csv; $c = if ($r[0].PSObject.Properties.Name -contains 'FrameTime') {'FrameTime'} else {'MsBetweenPresents'}
   $ft = $r | % { [double]$_.$c } | sort; $avg = ($ft | measure -Average).Average
   "avg {0:N1} fps, 1% low {1:N1} fps" -f (1000/$avg), (1000/$ft[[int]($ft.Count*0.99)-1])
   $r | group PresentMode | select Count, Name
   ```
   `PresentMode` should read `Hardware: Independent Flip` (or `Hardware Composed: Independent Flip`) for a borderless game. `Composed: Flip` means DWM is compositing it (extra latency).

Alternatives: Xbox Game Bar (Win+G > Performance > FPS) gives avg fps only. RTSS / MSI Afterburner gives fps, 1 % low and a frametime graph, but not the present mode.

| # | Check | How | Result |
|---|---|---|---|
| G1 | Borderless game, spike off | PresentMon `off.csv` | avg ___ fps, 1% low ___ fps, mode ___ |
| G2 | Borderless game, spike on + sharing | PresentMon `on.csv` | avg ___ fps, 1% low ___ fps, mode ___ |
| G3 | Spike stats during G2 | `s` in the spike: captured/presented per second, max-gap, late, gpu-copy | |
| G4 | Exclusive-fullscreen game | The Area over the game. Is the Virtual monitor (and the viewer) **mirrored**, **black**, or **stalled** (last frame frozen)? Alt+Tab in and out: does it recover? | |
| G5 | Game focus | The spike's window never takes focus: no minimising, no lost input while it runs. | |

## Delay, HDR, bandwidth

| # | Check | How | Result |
|---|---|---|---|
| T1 | End-to-end delay | Put a millisecond stopwatch (e.g. https://www.online-stopwatch.com/full-screen-stopwatch/ or any ms timer) inside the Area. The viewer places their screen next to yours, or shares a photo/video of both. Photograph both screens in one shot, 3–5 times, and record the differences. | ___ ms, ___ ms, ___ ms |
| H1 | HDR stays off | Settings > Display > the Virtual monitor: "Use HDR" is off or unavailable. The real monitor's HDR is off too (the spike captures SDR BGRA8). | |
| H2 | No washed-out colours | The viewer compares colours with a reference (the test pattern's red and green, a known image). | |
| W1 | Resolution under load | Stream 5 minutes of heavy motion (a game, or a fast video in the Area). Does Discord's sent resolution or fps drop (Debug panel)? To provoke it, start a large download at the same time. | |
| W2 | Encoder | The Debug panel's encoder / codec line (AMF / H.264 / AV1 ...). | |

## Undo

1. Quit the spike (`q`). The Virtual monitor disappears.
2. Elevated PowerShell: `powershell -ExecutionPolicy Bypass -File spike\driver\uninstall.ps1`. It removes the device node and the driver package, and deletes the certificate with thumbprint `3C918FC73525AD8B1521B6DB26B71F694277CC49` from Root and TrustedPublisher. It ends with "SudoVDA and its certificate are gone". It refuses to run if Apollo is installed (Apollo shares the driver), unless you pass `-Force`.
3. Settings > Display: only the 5120×1440 monitor is left, at its old position.

| # | Check | Result |
|---|---|---|
| U1 | `uninstall.ps1` ends green | |
| U2 | `certlm.msc` (local machine certificates): no `sudovda@su.mk` in Trusted Root or Trusted Publishers | |
| U3 | Device Manager > Display adapters: no SudoVDA | |
