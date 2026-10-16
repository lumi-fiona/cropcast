# How Discord shares a Virtual monitor

Research for [#3](https://github.com/lumi-fiona/cropcast/issues/3), part of map [#1](https://github.com/lumi-fiona/cropcast/issues/1). Researched 2026-10-16.

Question: how does Discord's Windows **Screen** share behave when it's pointed at cropcast's Virtual monitor (an IddCx virtual display sized exactly to the Area)? This covers audio, resolution changes mid-stream, the capture API, and stream ceilings.

Evidence levels used below:

- **Confirmed**: stated by Discord or Microsoft in their own docs, blog or patch notes.
- **Reported**: users in forums or issue trackers, client logs people posted, or third-party client source code. Credible, but not first-party.
- **Inference**: my own reasoning from the above. Not verified.

## Short answer

1. **Audio:** a Screen share on the Windows desktop client can carry **system audio** (the "share system audio" option). It's a system loopback, not tied to a monitor, so it should work the same for a Virtual monitor. Known catch: it also captures **Discord's own output**, so people in the call can hear themselves echo unless they use headphones or move Discord's output to another device.
2. **Mid-stream resolution change:** nothing first-party says what happens. Both Windows capture APIs force the capturer to rebuild when the mode changes. Whether Discord then re-sizes the stream, rescales into the old size, freezes or drops the share is **not documented**. It's the biggest open item and needs a hands-on test.
3. **Capture API:** Discord uses several OS methods with automatic fallback (Confirmed). For *windows* on Windows 11 the default is Windows Graphics Capture (WGC) (Confirmed). For *whole screens* Discord doesn't name the API. The Developer Mode debug panel shows frame counts per capture method, so a test can settle it. IddCx displays are normal displays to DWM, so both DXGI Desktop Duplication and WGC can capture them (Inference, backed by OBS/Sunshine practice on VDD/SudoVDA).
4. **Ceilings:** free accounts get 720p/30. Nitro gets up to "4K/60" (Confirmed). Boosted servers raise the floor for everyone (L1 720p60, L2 1080p60) (Confirmed). Discord's presets are 16:9 **boxes** (e.g. 1440p = 2560×1440, max 3,686,400 px). An Area larger than the chosen box gets downscaled, and an Area that fits inside it probably isn't (Reported + Inference). Bandwidth adaptation can still lower resolution on the fly (Confirmed in principle). Keep the Virtual monitor **SDR**, because HDR shares come out washed out (Reported, long-standing).

## 1. Audio

### Confirmed

- Discord's Go Live article: "Currently audio can only be captured by the Windows desktop, MacOS desktop, Chrome browser, and mobile clients." ([Discord support, Go Live and Screen Share](https://support.discord.com/hc/en-us/articles/360040816151-Go-Live-and-Screen-Share))
- Discord's own blog says "Share Entire Screen" shares "everything you see and hear, including … — if you choose to share your system audio — all the sounds from your applications". "Share Application" shares only that app's audio. ([Discord blog, How to Stream to Discord](https://discord.com/blog/how-to-stream-to-discord-from-desktop-or-mobile), Dec 2024)
- For *application* shares Discord captures audio "from the shared process and all its children" using OS audio APIs. ([Discord engineering blog, How It All Goes Live](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology), Mar 2024)
- Since Aug 2024 the audio toggle can be changed mid-stream without restarting. ([Discord Patch Notes Aug 30 2024](https://discord.com/blog/discord-patch-notes-august-30-2024))

### Reported

- Screen-share audio arrived on Windows around Jan 2024 (macOS late Dec 2023). Before that a Screen share had **no** audio at all. ([Reddit r/discordapp, Jan 2024](https://www.reddit.com/r/discordapp/comments/198cbqa/discord_sharing_whole_screen_now_also_shares_audio/); [TroubleChute](https://hub.tcno.co/videos/screenshare-desktop-with-audio-new-discord-update-info-guide/); older behaviour in [HN, 2023](https://news.ycombinator.com/item?id=36431940))
- Whole-screen audio **includes Discord's own voice output**, so call members hear themselves echo. Discord feedback from Feb 2024 and a Reddit thread from May 2025 say there's no built-in exclusion. ([Discord feedback](https://support.discord.com/hc/en-us/community/posts/21594139117847-Screen-sharing-audio); [Reddit](https://www.reddit.com/r/discordapp/comments/1kjfvel/how_to_prevent_screenshare_from_also_sharing/)) A third-party client fixes this with a separate "per-window anti-echo" swap, which again suggests stock Discord uses plain system loopback. ([discordmaxxer commit](https://github.com/MaxxTopia/discordmaxxer/commit/c5ecf78cc4aacfd3da377531102ee9c936916ea4))

### Inference

- System-audio capture is a loopback of the default output device, so it doesn't depend on which monitor is shared. It should behave the same for the Virtual monitor. cropcast doesn't need to do anything for audio.
- Echo is Lumi's own concern to manage (headphones, or Discord's output on a different device). cropcast can't fix it and shouldn't try.

## 2. Mid-stream resolution change

### Confirmed (Windows API behaviour, not Discord's)

- DXGI Desktop Duplication: `AcquireNextFrame` returns `DXGI_ERROR_ACCESS_LOST` on a **mode change**, and the app "must release the IDXGIOutputDuplication interface and create a new" one. ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe))
- WGC: frames report a new `ContentSize` and the app must call `Direct3D11CaptureFramePool.Recreate` with the new size. ([Microsoft Learn](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.recreate)) OBS's display capture code shows the recreate-on-reset loop in practice. ([OBS commit 80b4a65](https://github.com/obsproject/obs-studio/commit/80b4a65cd2e7fe2cfd12efa4d8a0778958c739fe))
- Discord says its capture "has a robust fallback system so if one method fails, it will quickly switch to the next method so the stream isn't interrupted." ([Discord engineering blog](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology))

### Reported

- No report found of Discord ending a share because the display mode changed. Freezes on *window* resize are reported for Linux/PipeWire clients, which don't apply here. ([discord-screenaudio #220](https://github.com/maltejur/discord-screenaudio/issues/220))
- A Feb 2024 regression made ultrawide streams look "squished" unless the quality was set to **Source**. ([Reddit](https://www.reddit.com/r/discordapp/comments/1asti8a/screenshare_ratio_suddenly_squished/)) So aspect handling has broken before.

### Unknown → hands-on test

- Does the stream follow the new size (the encoder renegotiates), keep the old frame size with the picture scaled/letterboxed into it, freeze, or end?
- How long is the glitch (black frames, a frozen frame) per resize?
- Does it differ between capture methods (DD vs WGC) and between Source and a fixed preset?
- **Design hint (Inference):** if each resize costs a visible glitch, cropcast should apply the new Virtual monitor mode **once, when Edit mode ends** (hotkey released), not continuously while dragging. The glossary already models it that way ("Releasing the hotkey commits the change").

## 3. Capture API and virtual-monitor issues

### Confirmed

- Discord "uses multiple techniques" and, on Windows 11+, defaults to **Windows Graphics Capture** for *application windows*, falling back for full-screen-exclusive games. ([Discord support, Capturing Application Window](https://support.discord.com/hc/en-us/articles/9410427556375--Windows-Capturing-Application-Window-for-Screen-Share-and-Go-Live))
- With **Developer Mode** on, the Voice/Video debug panel's *Screen Share* tab shows "the number of frames captured for each screen share method". This is the way to see which API captures the Virtual monitor. (Same article.)
- Codecs: VP8 and H.264, plus HEVC and AV1 on some platforms. Hardware encoders are preferred. (Discord engineering blog)

### Reported

- The desktop client's Voice & Video settings have toggles like "Use our latest technology to capture your screen" and "Use Windows.Graphics.Capture to capture a window". Users flip them to fix black screens. ([Reddit 2022](https://www.reddit.com/r/discordapp/comments/xzpgmr/no_character_voices_when_sharing_screen_with_a/); [MSFS forums](https://forums.flightsimulator.com/t/streaming-msfs-in-discord-only-shows-black-screen/531052))
- Virtual-display projects advertise use with "any desktop sharing software", and people share VDD displays in Discord/Teams/Zoom. The ultrawide request in [VDD #488](https://github.com/VirtualDrivers/Virtual-Display-Driver/issues/488) (a 5120×1440 G9 user, same setup as Lumi) assumes sharing a virtual display in Discord works. I found **no** issue reporting a Discord black screen specific to an IddCx monitor. VDD black-screen issues are about the user's own view when the physical display is turned off. ([VDD #286](https://github.com/VirtualDrivers/Virtual-Display-Driver/issues/286), [#50](https://github.com/itsmikethetech/Virtual-Display-Driver/issues/50))
- **HDR:** sharing an HDR display gives SDR viewers a washed-out picture. This has been reported for 5+ years and is unfixed. ([Discord feedback, HDR Screen Share](https://support.discord.com/hc/en-us/community/posts/360044838292-HDR-Screen-Share); [discord-api-docs #6257](https://github.com/discord/discord-api-docs/discussions/6257)) An NVIDIA driver regression (545.84) also washed out streams, fixed in 546.31. ([Discord support](https://support.discord.com/hc/en-us/articles/18621944634391-Color-Issues-in-Go-Live-Screenshare-for-NVIDIA-Graphics-Card))

### Inference

- An IddCx monitor is a normal DWM output, so DD and WGC both work on it. OBS, Sunshine and Apollo capture VDD/SudoVDA/parsec-vdd displays daily. Expect Discord to capture it too, but confirm in the debug panel.
- The Virtual monitor should advertise **SDR only** (no HDR modes). That avoids the washout and keeps the mirrored pixels identical.
- WGC draws a yellow capture border around the *captured* item on screen. On the Virtual monitor nobody looks at the screen itself, and the border isn't part of the captured frames, so it doesn't matter either way.

## 4. Stream ceilings and "pixel for pixel"

### Confirmed

| Who | Ceiling |
|---|---|
| Free account | 720p / 30 fps |
| Nitro (and Nitro Classic) | up to "4k/60fps" |
| Server boost L1 / L2 / L3 (for everyone streaming there) | 720p60 / 1080p60 / 1080p60 |

Sources: [Go Live and Screen Share](https://support.discord.com/hc/en-us/articles/360040816151-Go-Live-and-Screen-Share), [Server Boosting FAQ](https://support.discord.com/hc/en-us/articles/360028038352-Server-Boosting-FAQ), [Account/Server Caps](https://support.discord.com/hc/en-us/articles/33694251638295-Discord-Account-Caps-Server-Caps-and-More). Quality can be changed mid-stream. Max 50 viewers.

- The encoder's resolution/framerate/quality follows **estimated bandwidth**, and the stream is held to what the slowest viewer can take. Under congestion it drops frames and adjusts quality. (Discord engineering blog) So even a correctly sized stream can be **downscaled live** by bandwidth adaptation.

### Reported

- Discord client logs show each preset as a fixed 16:9 **box plus a pixel budget**. For example 720p → `maxResolution {type:"fixed", 1280×720}, maxPixelCount 921600`, 1080p → 1920×1080 / 2,073,600, 1440p → 2560×1440 / 3,686,400, with bitrate caps around 3.5–9 Mbps. ([Arch forums log](https://bbs.archlinux.org/viewtopic.php?pid=2290235); [omarchy #2044](https://github.com/omacom/omarchy/issues/2044); [Vesktop #191](https://github.com/Vencord/Vesktop/issues/191)) Third-party clients build these boxes as `height × 16/9`. ([Vesktop ScreenSharePicker](https://github.com/Vencord/Vesktop/blob/ac05783f/src/renderer/components/ScreenSharePicker.tsx))
- Preset choices in the client: 480/720/1080/1440/Source, at 15/30/60 fps. (Vesktop source; Vencord fakeNitro docs)
- Non-16:9 sources are sent **with their own aspect** and letterboxed on the viewer side. 21:9 and 32:9 users get bars top and bottom ([Discord feedback 2020](https://support.discord.com/hc/en-us/community/posts/360059654751-Aspect-Ratio-Settings-for-Go-Live), [2022](https://support.discord.com/hc/en-us/community/posts/4641557944727--Feature-Request-Allow-users-with-ultrawide-monitors-to-stream-in-16-9-ratio)). That's the bad-looking whole-screen share cropcast exists to avoid.

### Inference: what this means for the Area

- An Area that **fits inside the chosen box** (e.g. 2381×1307 inside 1440p's 2560×1440, 3.11 MP < 3.69 MP) should not need scaling. One that doesn't fit (e.g. under the 1080p preset) is scaled down with its aspect kept. **Source** should send native size up to the Nitro cap, but where Source's cap sits (4K pixel budget?) is undocumented.
- So "pixel for pixel" needs **Nitro + Source (or 1440p with Area ≤ 2560×1440) + enough bandwidth**. Free accounts can never get it for Areas larger than 1280×720.
- No bars get *baked in*: the stream carries the Area's own aspect, and the viewer's player letterboxes. That fits cropcast's rule.
- **Odd dimensions:** H.264/HEVC 4:2:0 needs even width and height. An Area like 2381×1307 may be cropped or padded by a pixel, or scaled, by the encoder. cropcast should probably **round the Virtual monitor to even dimensions** (cheap insurance). Test whether odd sizes come through untouched.

## Hands-on test list (for the spike)

Run with two accounts (streamer and a viewer on another machine), Developer Mode on, a test pattern on the Area (1-px checkerboard plus a ruler) so scaling is visible.

1. **Shows up:** the Virtual monitor appears in Discord's Screens list. Which capture method counts frames in the debug panel (DD vs WGC)?
2. **Pixel for pixel:** at Source / Nitro, does a 2381×1307 Virtual monitor arrive at 2381×1307? Check the viewer's stream info / debug panel resolution and look for checkerboard moiré. Repeat at 1440p and 1080p presets, and with an Area larger than 2560×1440.
3. **Odd vs even sizes:** 2381×1307 vs 2380×1306. Any crop, pad or scale?
4. **Mid-stream resize:** change the Virtual monitor mode while live (small and large steps, aspect changes). Does it follow, rescale, freeze or end? Measure glitch duration. Try 10 resizes in a row.
5. **Remove / re-add:** Sharing off → on recreates the Virtual monitor. Does Discord's share survive, or must Lumi re-pick the screen? (Matters for the Sharing on/off UX.)
6. **Audio:** system audio arrives while sharing the Virtual monitor. Confirm the Discord-voice echo and whether any setting excludes it.
7. **HDR:** confirm the Virtual monitor stays SDR. If HDR is on for the physical monitor, check that the mirrored Area doesn't wash out.
8. **Bandwidth:** does Discord drop resolution under load (watch the viewer resolution over ~5 min of motion)?
9. **60 fps:** does the viewer get a steady 60 at the Area's size with the encoder in use (NVENC/AMF/QSV)?
