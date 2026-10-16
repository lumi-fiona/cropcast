# App filtering: hiding or allow-listing programs in the share

Research for issue #9 (map: #1). Researched 2026-10-16 against Microsoft Learn (including the MicrosoftDocs source repos), and the source of OBS Studio and Invisiwind. Vocabulary follows [GLOSSARY.md](../../GLOSSARY.md); the capture route is [ADR 0001](../adr/0001-capture-route.md) and the pipeline is the mirror-pipeline research (branch `research/mirror-pipeline`). **[test]** marks anything only a hands-on test on Lumi's PC can settle.

## Answer

| | Verdict | Recommended mechanism | Key limits | Rough cost |
|---|---|---|---|---|
| **Blacklist** | Possible, with caveats | **Mask in the copy**: each frame, work out the visible part of every blacklisted window inside the Area and paint it black on the swapchain back buffer after the copy | Viewers see black, not what is behind the window. A window being dragged or animating can leak a thin edge for a frame unless the mask is padded **[test]**. Owned popups, menus and tooltips count only if they belong to the same process | One shared "filter" module, a few hundred lines of Rust. GPU cost about zero (one `ClearView` with a rect list). No injection, so no anti-cheat exposure |
| **Whitelist** | Possible, with caveats | **The same mask, inverted**: keep the visible parts of allow-listed windows, paint the rest of the Area black | Anything not allow-listed disappears, including shell UI (taskbar, Start, notifications, drag images) and popups owned by other processes. An allowed window covered by another window shows a black hole, not its hidden content. Same edge-leak risk while windows move **[test]** | Shares ~90% of the blacklist code; a mode switch and a list. Same near-zero GPU cost |

Both directions use one per-frame **region**: "pixels that belong to a listed program and are on top". Blacklist blacks out that region; whitelist blacks out its complement. Neither touches another process, so both use only public, documented APIs that need no rights over the game.

Not recommended for v1:

- **Injected `WDA_EXCLUDEFROMCAPTURE`** (what Invisiwind does): the cleanest picture for a blacklist (viewers see what is behind the window, no lag), but it writes into other processes. See [Injection](#injection-wda_excludefromcapture-on-another-processs-window). Keep it as a possible opt-in later, never a default.
- **Per-window WGC compositing** for a whitelist: N capture sessions, popups clipped to the main window, minimized windows go stale, and the result is not what Lumi sees. See [Per-window compositing](#per-window-compositing-whitelist-alternative).

## Why `WDA_EXCLUDEFROMCAPTURE` can't just be set on other apps

- `SetWindowDisplayAffinity`'s `hWnd` is "a handle to the top-level window. **The window must belong to the current process.**" `WDA_EXCLUDEFROMCAPTURE` makes the window "displayed only on a monitor. Everywhere else, the window does not appear at all" ([SetWindowDisplayAffinity][wda]).
- There is no documented cross-process variant. The documented neighbour, DWM thumbnails, has the same kind of rule: the destination "must either be the desktop window itself or be owned by the process that is calling... This is required to prevent applications from affecting the content of other applications" ([DwmRegisterThumbnail][thumb]). Microsoft's stance is consistent: one process does not change how another's windows are captured.
- The affinity "is not a security feature or an implementation of DRM" and only works while DWM composes the desktop ([SetWindowDisplayAffinity remarks][wda]); always true on Windows 11.

So the only way to apply it to another app is to run code inside that app.

## Injection: `WDA_EXCLUDEFROMCAPTURE` on another process's window

How the known tool does it: **Invisiwind** ("hide certain windows when sharing your full screen") "performs dll injection to SetWindowDisplayAffinity to `WDA_EXCLUDEFROMCAPTURE`" ([README][inv-readme]). Its current Rust source does not load a DLL; it opens the target with `PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_VM_READ` and calls `CreateRemoteThread` at user32's `SetWindowDisplayAffinity` with `(hwnd, WDA_EXCLUDEFROMCAPTURE)`, then reads it back with `GetWindowDisplayAffinity` to check it took ([remote.rs][inv-remote], [native.rs][inv-native]). 32-bit targets need a matching 32-bit helper, because export addresses are per architecture ([native.rs][inv-native]).

Risks and limits:

1. **Elevated targets.** Mandatory Integrity Control: "a principal with a low integrity level cannot write to an object with a medium integrity level", elevated processes run at high, and objects default to `NO_WRITE_UP` ([MIC][mic]). A normal (medium) cropcast cannot get the write/thread rights on an elevated app; it would have to run elevated itself.
2. **Protected processes** refuse `PROCESS_CREATE_THREAD`, `PROCESS_VM_WRITE` and `PROCESS_VM_OPERATION` from any process ([process access rights][proc]). Anti-cheat-protected games behave similarly in practice **[test]**, though blacklisting a game is not the use case.
3. **UWP / packaged apps**: their top-level frame belongs to a host process, not the app; which process to inject and whether the affinity sticks is **[test]**.
4. **Antivirus**: Invisiwind's own README warns it "may be detected as a virus/trojan by your antivirus due to the fact that it performs dll injection" ([README][inv-readme]).
5. **Anti-cheat**: vendors don't publish their rules. The best public signal is OBS: its Game Capture ships a separate "Use anti-cheat compatibility hook" mode ([en-US.ini][obs-ini]) that injects with `SetWindowsHookEx(WH_GETMESSAGE, ...)` instead of `CreateRemoteThread` ([inject-library.c][obs-inject], [game-capture.c][obs-gc]). A mainstream, signed app still had to avoid `CreateRemoteThread` to get along with anti-cheats. Whether a kernel anti-cheat flags cropcast writing into Discord or a browser (not the game) while a protected game runs is **[test]**, and the risk is a ban on Lumi's account, which cannot be undone by a patch.
6. **Global side effect**: the window vanishes from every capture (Discord's own window share, OBS, Snipping Tool), not just cropcast's, until reset or the window closes.
7. **New windows**: each new top-level window needs a new injection. Invisiwind's FAQ: "Do future instances of the application get automatically hidden? No" ([README][inv-readme]).

Verdict: the best picture, the worst risk profile. Not for v1; at most a later opt-in, off by default, with a warning about anti-cheat.

## Masking in the copy (recommended for both)

Where it slots into the ADR 0001 pipeline: after the one `CopySubresourceRegion` of the Area into the back buffer, and before `Present`, call `ID3D11DeviceContext1::ClearView(rtv, black, rects, n)`. It takes an array of rectangles on a render-target view and "the runtime drops invalid calls. Empty rectangles... are a no-op" ([ClearView][clearview]). No shader, no extra texture.

### Computing the region each frame

1. **Enumerate top-level windows in z-order** (top first): `EnumWindows`, or walk with `GetWindow(GW_HWNDNEXT)` ("the window below the specified window in the Z order") ([GetWindow][getwindow]). Skip windows that are not visible (`IsWindowVisible`), minimized (`IsIconic`), or cloaked (`DWMWA_CLOAKED`: cloaked by the app, the shell, or inherited from the owner, e.g. other virtual desktops) ([DWMWINDOWATTRIBUTE][dwmwa]).
2. **Use the real on-screen bounds**: `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` returns "the extended frame bounds rectangle in screen space" ([DWMWINDOWATTRIBUTE][dwmwa]). `GetWindowRect` "may include invisible resize borders" and would over-mask; the same page notes that, unlike the window rect, the extended frame bounds "are not adjusted for DPI" ([GetWindowRect][getwindowrect]), so cropcast must run per-monitor DPI aware to get physical pixels that match the capture.
3. **Classify** each window as listed or not by its program (see [Identifying a program](#identifying-a-program)).
4. **Build the region top-down** with GDI regions (native, no math to write): keep `covered` = union of everything above; for a listed window, `visible = rect − covered`; add `visible` to the mask; then `covered ∪= rect`. `CombineRgn` with `RGN_DIFF` / `RGN_OR` does the set math and `GetRegionData` returns the result as a list of rectangles ([CombineRgn][combinergn], [GetRegionData][getregiondata]).
5. **Blacklist**: clear the mask rects. **Whitelist**: clear the Area minus the mask (`RGN_DIFF` against the Area rect).
6. Clip to the Area and offset into back-buffer coordinates.

Occlusion falls out of step 4: a blacklisted window behind a normal window is masked only where it shows; a whitelisted window behind a non-listed one shows only where it is on top.

### Per-frame accuracy and moving windows

- A WGC frame carries `SystemRelativeTime`, "the QPC time at which the compositor rendered the frame" ([SystemRelativeTime][srt]); window positions are read later, in `FrameArrived`. If a window moved in between, the mask is for a slightly newer position than the pixels. At 60 fps that gap is up to one frame **[test]** for how many pixels this leaks during a fast drag.
- Mitigations, cheapest first:
  1. **Pad** each listed rect by a few pixels (blacklist) or shrink it (whitelist).
  2. **Union of last and current rect** for each listed window, so a move covers both positions.
  3. **While a move/resize is in progress** (`EVENT_SYSTEM_MOVESIZESTART` / `EVENT_SYSTEM_MOVESIZEEND`, "A window is being moved or resized" ([event constants][events])), blacklist the window's whole travel box, or for whitelist hold the last good mask.
- **DWM animations** (open, close, minimize, restore) draw the window in places its bounds don't report. A blacklisted window restoring from the taskbar may flash at full or partial size for a few frames before its rect is valid **[test]**. Cheap guard: treat `EVENT_SYSTEM_MINIMIZEEND` ([event constants][events]) and the first frames after `EVENT_OBJECT_SHOW` for a listed program as "black the whole Area" for ~200 ms. Whether `DWMWA_TRANSITIONS_FORCEDISABLED` can be set on another app's window via `DwmSetWindowAttribute` ([DWMWINDOWATTRIBUTE][dwmwa]) is **[test]**; do not count on it.
- Shadows, rounded corners and semi-transparent windows: the extended frame bounds exclude the drop shadow, so a blacklisted window's faint shadow may show; harmless. A translucent blacklisted window is masked as an opaque rectangle.

### Finding new windows

Re-enumerating every frame is simplest and is correct by construction: new windows, closed windows and z-order changes are picked up on the next frame. Cache "pid → listed?" so the program lookup runs once per process. Cost of `EnumWindows` at 60 Hz with a few hundred windows **[test]** (expected well under 1 ms).

If that turns out too slow, switch to events: `SetWinEventHook(..., WINEVENT_OUTOFCONTEXT)` with process and thread 0 receives events "from all threads and processes", delivered on the calling thread, which "must have a message loop"; out-of-context means no DLL is loaded into anyone ([SetWinEventHook][wineh]). Useful events: `EVENT_OBJECT_CREATE` / `SHOW` / `DESTROY`, `EVENT_OBJECT_LOCATIONCHANGE` (sent for windows, "not generated for any children"), `EVENT_OBJECT_REORDER`, `EVENT_OBJECT_CLOAKED` / `UNCLOAKED` ([event constants][events]).

### What the viewer sees where a window was

| Option | How | Notes |
|---|---|---|
| **Black** (recommended) | `ClearView` with the rect list | Zero cost, unambiguous, never leaks content. |
| Blur or tint | A pixel-shader pass over the rects | Adds a pass and a shader; a light blur can still leak big text, so it must be heavy. |
| Last frame / freeze | Keep the last unmasked pixels per rect in a persistent texture | Shows stale content that may be wrong or itself sensitive; not worth it. |
| What is behind the window | Not possible with masking: the monitor capture never contains occluded pixels. Only the injected affinity gives this. | |

Optional nicety later: a solid colour or pattern instead of black, so viewers read it as "hidden on purpose".

## Per-window compositing (whitelist alternative)

`IGraphicsCaptureItemInterop::CreateForWindow` captures one window (Windows 10 1903+) ([CreateForWindow][cfw]). Building the whitelist from N such captures, each drawn at its screen position:

- **Popups and menus**: by default only the main window. `GraphicsCaptureSession.IncludeSecondaryWindows` (Windows 11 24H2, build 26100, which Lumi has) adds `WS_POPUP` / `WS_EX_TOOLWINDOW` windows that intersect the main window, but they "are clipped if they go outside the bounds of the main top level window" ([IncludeSecondaryWindows][isw]). A dropdown or context menu that spills out is cut.
- **Minimized windows**: no new frames. OBS's window capture skips the tick while `IsIconic` or invisible, and notes "If HWND is invisible, WGC module can't be initialized successfully" ([window-capture.c][obs-wc]); its WGC helper also checks iconic twice around the client-rect read ([winrt-capture.cpp][obs-winrt]).
- **Z-order** must be rebuilt by hand from enumeration, and each window's frames arrive on their own schedule, so windows can be out of step with each other **[test]**.
- **Cursor**: per-window captures draw it only over that window **[test]**.
- **Cost**: N capture sessions and N copies per frame, N frame pools to manage, a capture border to suppress on each (OBS sets `IsBorderRequired(false)` per session ([winrt-capture.cpp][obs-winrt])).
- **Upside**: an allowed window covered by another app is still shown whole. But that is a picture Lumi does not see on her own screen, which conflicts with the Area being "pixel for pixel" what she sees.

DWM thumbnails (`DwmRegisterThumbnail` into the presenter window, top-level sources only ([DwmRegisterThumbnail][thumb])) would be a cheaper way to do the same composition with no copies, but share the same z-order, popup and cursor problems; ordering of multiple thumbnails and their behaviour for minimized sources are **[test]**.

Verdict: more code and more surprises than the inverted mask, for a picture that differs from Lumi's screen. Not recommended.

## Identifying a program

- **Primary key: full exe path.** `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `QueryFullProcessImageNameW`, which accepts a handle with `PROCESS_QUERY_LIMITED_INFORMATION` ([QueryFullProcessImageNameW][qfpin]). That right gives "a subset of the information" of `PROCESS_QUERY_INFORMATION` and, unlike it, is not on the list of rights refused to callers of protected processes ([process access rights][proc]). Whether a medium-integrity cropcast gets it on elevated apps and anti-cheat-protected games **[test]**; if it fails, treat the window as "unknown program" (not listed).
- **Display name / UI**: match on exe file name as a fallback for programs that update to versioned paths (Discord's `app-1.0.xxxx` folders, for example **[test]**). Show the user name + icon, store the path.
- **UWP / packaged apps**: the frame window is hosted by another process, so the exe path is the host's, not the app's **[test]**. Use the AppUserModelID instead: `GetApplicationUserModelId` takes a process handle with `PROCESS_QUERY_LIMITED_INFORMATION` ([GetApplicationUserModelId][aumid]); which process to ask (the frame host or the app's own content window) **[test]**.
- **Window class** is not a good primary key (many apps share Chromium/Electron classes) but can refine a match, e.g. "only Discord's main window".
- **Popups, menus, tooltips** belong to the process that created them, so a process-level match includes them automatically. Things drawn by *other* processes on an app's behalf (shell dialogs, IME candidate windows, some system context menus) are not included **[test]** for the ones Lumi cares about.

## Limits shared by every approach

- **The Virtual monitor is outside the filter.** Discord captures the Virtual monitor from DWM, and anything that lands on it is seen (ADR 0001, Consequences). A blacklisted window dragged onto the Virtual monitor is visible to viewers. The presenter is topmost, but another topmost window can still sit above it. Mitigation is the ADR's placement rule, plus optionally: if a blacklisted window intersects the Virtual monitor, black the whole presenter **[test]** for how often this happens.
- **Exclusive-fullscreen** games may already mirror black (ADR 0001); filtering does not change that.
- **cropcast's own windows** (Border) are already excluded via `WDA_EXCLUDEFROMCAPTURE` on its own process, which is the supported use of the flag ([SetWindowDisplayAffinity][wda]).

## Cost summary

- **Masking (both modes)**: one window enumeration + region build per frame on the CPU (expected sub-millisecond **[test]**), one `ClearView` on the GPU (negligible next to the ~1% budget in the mirror research). Code: a filter module (enumerate, classify with a pid cache, build region, emit rects) plus a list UI; blacklist and whitelist differ by one `RGN_DIFF`. Rough size: a few hundred lines.
- **Injection (not recommended)**: a 64-bit and a 32-bit helper, per-window injection on every new window, elevation for elevated targets, antivirus noise, unknown anti-cheat risk.
- **Per-window WGC (not recommended)**: N sessions, N copies, a compositor, z-order tracking, popup clipping.

## Only a hands-on test can settle

- How many pixels a fast drag or a restore animation leaks with masking at 60 fps, and how much padding hides it.
- `EnumWindows` + region build time per frame with Lumi's usual window count.
- `PROCESS_QUERY_LIMITED_INFORMATION` on elevated apps and on a running anti-cheat game.
- How UWP/packaged apps (e.g. Settings, Store apps) map to a program, and which process's AUMID to read.
- Whether Discord's and browsers' exe paths change on update.
- For injection only: whether any anti-cheat Lumi plays under reacts to `CreateRemoteThread` into another, unrelated process.

[wda]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
[thumb]: https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmregisterthumbnail
[mic]: https://learn.microsoft.com/en-us/windows/win32/secauthz/mandatory-integrity-control
[proc]: https://learn.microsoft.com/en-us/windows/win32/procthread/process-security-and-access-rights
[clearview]: https://learn.microsoft.com/en-us/windows/win32/api/d3d11_1/nf-d3d11_1-id3d11devicecontext1-clearview
[getwindow]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindow
[getwindowrect]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect
[dwmwa]: https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute
[combinergn]: https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-combinergn
[getregiondata]: https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-getregiondata
[srt]: https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframe.systemrelativetime
[events]: https://learn.microsoft.com/en-us/windows/win32/winauto/event-constants
[wineh]: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook
[cfw]: https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow
[isw]: https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.includesecondarywindows
[qfpin]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew
[aumid]: https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getapplicationusermodelid
[inv-readme]: https://github.com/radiantly/Invisiwind/blob/main/README.md
[inv-remote]: https://github.com/radiantly/Invisiwind/blob/main/src/remote.rs
[inv-native]: https://github.com/radiantly/Invisiwind/blob/main/src/native.rs
[obs-ini]: https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/data/locale/en-US.ini
[obs-inject]: https://github.com/obsproject/obs-studio/blob/master/shared/obs-inject-library/inject-library.c
[obs-gc]: https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/game-capture.c
[obs-wc]: https://github.com/obsproject/obs-studio/blob/master/plugins/win-capture/window-capture.c
[obs-winrt]: https://github.com/obsproject/obs-studio/blob/master/libobs-winrt/winrt-capture.cpp
