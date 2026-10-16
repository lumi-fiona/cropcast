# Research: which virtual display driver cropcast should use

Ticket: [#2 Pick the virtual display driver](https://github.com/lumi-fiona/cropcast/issues/2) (map: #1). Researched 2026-10-16 from primary sources: driver source, READMEs and issues on GitHub, and Microsoft Learn. Every claim links to its source. Points that only a hands-on test can settle are marked **[test]**.

## Recommendation

Use **SudoVDA** ([SudoMaker/SudoVDA](https://github.com/SudoMaker/SudoVDA), the driver inside Apollo). It is the only candidate whose control API matches cropcast's model:

- one IOCTL adds a monitor with an **arbitrary width × height × refresh rate**;
- one IOCTL removes it by GUID;
- a watchdog removes the monitor if the app dies, so Sharing-off leaves nothing behind.

Its license is permissive, and after install a non-admin process can drive it.

What SudoVDA does *not* do: change an existing monitor's resolution. A new Area size means remove, then add again. Whether Discord keeps sharing across that re-plug is the main open question **[test]**.

If Discord drops the share on a re-plug, the fallback is a small **cropcast fork of SudoVDA**: add one IOCTL that calls `IddCxMonitorUpdateModes`, and self-sign the package the same way Apollo does. Owning the binary is likely in any case, because SudoVDA publishes no release binaries of its own (see Signing).

Rejected:

- **VirtualDrivers/Virtual-Display-Driver (VDD)**: modes come from an XML file, and the only runtime control is a driver reload.
- **parsec-vdd**: a closed Parsec binary with no redistribution grant and at most 5 custom modes.
- **IddSampleDriver**: unsigned, with a fixed mode list.
- **Writing a driver from scratch**: SudoVDA already is that driver; fork it instead.

## Candidates compared

| | SudoVDA | VDD (MTT) | parsec-vdd | IddSampleDriver |
|---|---|---|---|---|
| Add/remove at runtime | `IOCTL_ADD_VIRTUAL_DISPLAY` / `IOCTL_REMOVE_VIRTUAL_DISPLAY` | Monitor count from `vdd_settings.xml` + `RELOAD_DRIVER` over a named pipe | `VDD_IOCTL_ADD` / `VDD_IOCTL_REMOVE` | Fixed at load |
| Arbitrary mode (e.g. 2381×1307) | Yes, passed in the add IOCTL | Only via an XML `<resolution>` list + reload | Presets + max 5 registry entries (HKLM) | No |
| Change mode of a live monitor | No (remove + re-add) | No (reload = all monitors drop) | Only among preset/registry modes | No |
| Keep-alive needed | Ping within watchdog (default 3 s, configurable, 0 = off) | No | Ping every <100 ms or all displays drop after ~1 s | No |
| Admin after install | No (device ACL grants Everyone read/write) | Writing `C:\VirtualDisplayDriver\vdd_settings.xml` + pipe | Registry modes in HKLM need admin | n/a |
| Signing | Self-signed `CN=sudovda@su.mk` (via Apollo), cert imported into Root + TrustedPublisher | Signed (SignPath Foundation certificate) | Signed by Parsec | Unsigned |
| License | MIT / CC0 (author's changes) + Microsoft sample | MIT | Wrapper MIT; **driver binary is Parsec's, no grant** | Microsoft sample |
| Activity | Last push 2025-04-14; Apollo last release v0.4.6 2025-07-13 | Last release 25.7.23 (2025-07-23) | Active | Stale |

Sources:

- SudoVDA: [sudovda-ioctl.h](https://github.com/SudoMaker/SudoVDA/blob/master/Common/Include/sudovda-ioctl.h), [Driver.cpp](https://github.com/SudoMaker/SudoVDA/blob/master/Virtual%20Display%20Driver%20(HDR)/SudoVDA/Driver.cpp), [SudoVDA.inf](https://github.com/SudoMaker/SudoVDA/blob/master/Virtual%20Display%20Driver%20(HDR)/SudoVDA/SudoVDA.inf), [README](https://github.com/SudoMaker/SudoVDA#readme).
- VDD: [Driver.cpp](https://github.com/VirtualDrivers/Virtual-Display-Driver/blob/master/Virtual%20Display%20Driver%20(HDR)/MttVDD/Driver.cpp), [vdd_settings.xml](https://github.com/VirtualDrivers/Virtual-Display-Driver/blob/master/Virtual%20Display%20Driver%20(HDR)/vdd_settings.xml), [README](https://github.com/VirtualDrivers/Virtual-Display-Driver#readme).
- parsec-vdd: [core/parsec-vdd.h](https://github.com/nomi-san/parsec-vdd/blob/main/core/parsec-vdd.h), [README](https://github.com/nomi-san/parsec-vdd#readme), sections "Using Core API" and "Known Limitations 2. Custom resolutions".
- Apollo: [install.bat](https://github.com/ClassicOldSong/Apollo/blob/master/src_assets/windows/drivers/sudovda/install.bat), [sudovda.cer](https://github.com/ClassicOldSong/Apollo/blob/master/src_assets/windows/drivers/sudovda/sudovda.cer).

## SudoVDA: the facts later tickets need

### Opening the device

- Device interface GUID: `SUVDA_INTERFACE_GUID = {e5bcc234-1e0c-418a-a0d4-ef8b7501414d}`. Hardware ID: `root\sudomaker\sudovda`. Display class GUID: `{4d36e968-e325-11ce-bfc1-08002be10318}`. Source: [sudovda-ioctl.h](https://github.com/SudoMaker/SudoVDA/blob/master/Common/Include/sudovda-ioctl.h).
- To open the device, Apollo calls `SetupDiGetClassDevs(&SUVDA_INTERFACE_GUID, …, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)`, then `SetupDiEnumDeviceInterfaces`, then `SetupDiGetDeviceInterfaceDetail`, then `CreateFile(DevicePath, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, …, OPEN_EXISTING, …)`. Right after install it retries with backoff from 20 ms to 320 ms. Sources: Apollo [third-party/sudovda/sudovda.h](https://github.com/ClassicOldSong/Apollo/blob/master/third-party/sudovda/sudovda.h) and [virtual_display.cpp `openVDisplayDevice`](https://github.com/ClassicOldSong/Apollo/blob/master/src/platform/windows/virtual_display.cpp).
- **No admin needed after install.** The INF sets the device security to `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;WD)`: SYSTEM and Administrators get full access, Everyone (`WD`) gets read and write. Every IOCTL is `FILE_ANY_ACCESS`. Source: [SudoVDA.inf](https://github.com/SudoMaker/SudoVDA/blob/master/Virtual%20Display%20Driver%20(HDR)/SudoVDA/SudoVDA.inf).
- Call `IOCTL_GET_PROTOCOL_VERSION` first and refuse to run on a mismatch, as Apollo's `CheckProtocolCompatible` does. The protocol version in the source is `0.2.1`.

### IOCTLs

All IOCTLs are `CTL_CODE(FILE_DEVICE_UNKNOWN, fn, METHOD_BUFFERED, FILE_ANY_ACCESS)`. Source: [sudovda-ioctl.h](https://github.com/SudoMaker/SudoVDA/blob/master/Common/Include/sudovda-ioctl.h).

| IOCTL | fn | In | Out |
|---|---|---|---|
| `IOCTL_ADD_VIRTUAL_DISPLAY` | 0x800 | `{UINT Width; UINT Height; UINT RefreshRate; GUID MonitorGuid; CHAR DeviceName[14]; CHAR SerialNumber[14];}` | `{LUID AdapterLuid; UINT TargetId;}` |
| `IOCTL_REMOVE_VIRTUAL_DISPLAY` | 0x801 | `{GUID MonitorGuid;}` | none |
| `IOCTL_SET_RENDER_ADAPTER` | 0x802 | `{LUID AdapterLuid;}` | none |
| `IOCTL_GET_WATCHDOG` | 0x803 | none | `{UINT Timeout; UINT Countdown;}` |
| `IOCTL_DRIVER_PING` | 0x888 | none | none |
| `IOCTL_GET_PROTOCOL_VERSION` | 0x8FF | none | `{u8 Major, Minor, Incremental; bool TestBuild;}` |

### Add-handler behaviour

Source: [Driver.cpp](https://github.com/SudoMaker/SudoVDA/blob/master/Virtual%20Display%20Driver%20(HDR)/SudoVDA/Driver.cpp), IOCTL switch at roughly lines 1497–1640.

- The only validation is `Width > 0 && Height > 0 && RefreshRate > 0`. A `RefreshRate` below 1000 is read as Hz and multiplied by 1000 (millihertz internally), so pass `60`.
- Adding a GUID that already exists does not create a second monitor. It returns the existing monitor's `{AdapterLuid, TargetId}` with success, which means a resize must **remove first, then add**.
- When no connector slots are free it returns `STATUS_TOO_MANY_NODES`. The slot count comes from the registry value `maxMonitors`, default 10.
- The EDID is generated from `MonitorGuid.Data1`, `SerialNumber` and `DeviceName`. With a fixed GUID, Windows sees the same monitor every time and remembers its position. Apollo uses this deliberately (README, "About Virtual Display"). **cropcast should use one fixed GUID.**

### Which modes a monitor exposes

Sources: `SudoVDAParseMonitorDescription` and `SudoVDAMonitorQueryModes` in [Driver.cpp](https://github.com/SudoMaker/SudoVDA/blob/master/Virtual%20Display%20Driver%20(HDR)/SudoVDA/Driver.cpp).

- Each monitor reports a built-in default list (`s_DefaultModes`: 800×600, 1280×720 and other common modes).
- It also reports the requested size scaled by 100, 50, 75, 125 and 150 %, each at the requested refresh rate and at twice that rate.
- The requested mode is marked as the preferred mode, so a newly added 2381×1307@60 monitor should come up at that size. Integer scaling of odd sizes (2381 × 50 / 100 = 1190) is harmless.
- **The mode list is fixed when the monitor is added.** The driver never calls `IddCxMonitorUpdateModes`; a grep finds 0 occurrences.

### Setting the mode and position from the app

Source: Apollo [virtual_display.cpp](https://github.com/ClassicOldSong/Apollo/blob/master/src/platform/windows/virtual_display.cpp), `createVirtualDisplay` and `changeDisplaySettings`.

1. Find the new monitor's GDI name (`\\.\DISPLAYn`) from the returned `{AdapterLuid, TargetId}`. Apollo retries `GetAddedDisplayName` with backoff from 20 ms to 320 ms; in practice that means `QueryDisplayConfig` plus `DisplayConfigGetDeviceInfo(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME)`.
2. Apply the mode with `ChangeDisplaySettingsExW(name, &devmode{dmPelsWidth, dmPelsHeight, dmDisplayFrequency}, NULL, CDS_UPDATEREGISTRY, NULL)`. Apollo then refines it with `SetDisplayConfig` for fractional refresh rates; cropcast only needs 60.
3. Position the monitor the same way with `DM_POSITION`, or with `SetDisplayConfig`.

### Watchdog

Sources: SudoVDA [README "Config"](https://github.com/SudoMaker/SudoVDA#config) and `RunWatchdog` in Driver.cpp; Apollo `startPingThread`.

- If the driver hears nothing for `watchdog` seconds (registry value, default 3, 0 = off), it removes **all** virtual monitors.
- Every IOCTL resets the countdown. Apollo pings every `Timeout*1000/3` ms (once a second by default).
- This gives cropcast "nothing left behind when it crashes" for free. It also means cropcast must not let the ping thread stall, for example behind a UI-thread hang.

### Render adapter

The driver renders on the GPU with the most VRAM unless the registry value `gpuName` or `IOCTL_SET_RENDER_ADAPTER` says otherwise. On a multi-GPU machine, point it at the GPU that drives the 5120×1440 panel so the mirror avoids a cross-adapter copy. On a single-GPU machine there is nothing to do. Sources: README "Config"; IOCTL handler.

### Registry config

Registry config lives under `HKLM\SOFTWARE\SudoMaker\SudoVDA`: `gpuName`, `maxMonitors`, `watchdog`, `sdrBits`, `hdrBits`. The driver reads it only when it loads (README "Config"), so the installer step (elevated) is the place to write any of it.

## Install, signing and redistribution

### What Apollo actually ships

[install.bat](https://github.com/ClassicOldSong/Apollo/blob/master/src_assets/windows/drivers/sudovda/install.bat) does the following:

```
certutil -addstore -f root            sudovda.cer
certutil -addstore -f TrustedPublisher sudovda.cer
nefconc --remove-device-node --hardware-id root\sudomaker\sudovda --class-guid 4D36E968-E325-11CE-BFC1-08002BE10318
nefconc --create-device-node --class-name Display --class-guid 4D36E968-E325-11CE-BFC1-08002BE10318 --hardware-id root\sudomaker\sudovda
nefconc --install-driver --inf-path SudoVDA.inf
```

The uninstall script is the `--remove-device-node` line on its own.

- **The certificate is self-signed.** I decoded `sudovda.cer`: subject and issuer are both `CN=sudovda@su.mk`, valid 2025-07-13 to 2030-07-13. The package therefore installs only because the script trusts that certificate machine-wide, as a root CA and as a trusted publisher.
- **Why a self-signed package can load:** it is a UMDF user-mode driver. Microsoft's rule that new drivers must be signed through the Dev Portal applies to *kernel-mode* drivers ([Driver Signing Policy](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-policy--windows-vista-and-later-)). An IDD has no kernel-mode component ([IDD model overview](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/indirect-display-driver-model-overview)): the INF installs Microsoft's own `WUDFRd.sys` as the service and puts `SudoVDA.dll` in `drivers\umdf`.
- The evidence that this works on retail Windows 11 is Apollo's install base (about 11k stars, shipping this script). That is evidence of practice, not a Microsoft statement. **[test]** on Lumi's machine with Secure Boot on.
- **The binary has no standalone release.** The SudoVDA repo has 0 releases and 0 tags. Apollo's repo gitignores `*.dll`, so `SudoVDA.dll` exists only inside the Apollo installer (v0.4.6, 2025-07-13, the same date the certificate was issued).
- Two ways to get a binary:
  - **(a) Extract** the signed `SudoVDA.dll`, `.inf`, `.cat` and `.cer` from the Apollo 0.4.6 installer.
  - **(b) Build SudoVDA from source in CI** (WDK) and sign the catalog with **cropcast's own** self-signed certificate.

  Option (b) is cleaner for a portfolio project and is required for the fork fallback. It also avoids planting a third party's root certificate on the machine.
- **Security note for the install-flow ticket:** importing a certificate into `Root` makes Windows trust anything its private key signs. If cropcast signs its own build, generate a code-signing-only certificate, keep the key out of the repo (a CI secret), and remove the certificate on uninstall.
- **Signing it "properly" instead** means attestation signing. That needs an EV code-signing certificate to open a Partner Center hardware account, then submitting a CAB signed with that certificate ([Attestation signing](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-attestation)). That is a paid yearly EV certificate plus a company or identity verification. It is overkill for a single-user portfolio app.
- **Installing without `nefconc`:** nefcon is MIT ([nefarius/nefcon](https://github.com/nefarius/nefcon), v1.21.0) and fine to bundle. The same steps can also be done natively with SetupAPI from the elevated Rust helper: `SetupDiCreateDeviceInfo`, then `SetupDiSetDeviceRegistryProperty(SPDRP_HARDWAREID)`, then `SetupDiCallClassInstaller(DIF_REGISTERDEVICE)`, then `UpdateDriverForPlugAndPlayDevicesW`. This is what devcon and nefcon do internally. Not verified line by line here.

### Licenses

- **SudoVDA:** "MIT and CC0 or Public Domain (for changes I made, please consult Microsoft for their license), choose the least restrictive option" ([README "License"](https://github.com/SudoMaker/SudoVDA#license)). The underlying code is Microsoft's Indirect Display sample from Windows-driver-samples. Bundling the driver with an MIT exe is fine; ship a THIRD_PARTY_NOTICES entry for SudoVDA and the Microsoft sample.
- **Apollo** is GPL-3.0, but cropcast would take only the SudoVDA parts, which have their own license. Same author (Yukino Song / ClassicOldSong).
- **VDD:** MIT.
- **parsec-vdd:** the repo's MIT license covers nomi-san's wrapper and app only. The driver (`mm.dll`, `mm.inf`, `mm.cat`) is Parsec's, downloaded from `builds.parsec.app`, with no redistribution grant in the repo. Not shippable.

## Why not the others

- **VDD:**
  - Resolutions come from `<resolutions>` in `C:\VirtualDisplayDriver\vdd_settings.xml`, and the monitor count from `<monitors><count>`.
  - The named pipe `\\.\pipe\MTTVirtualDisplayPipe` accepts `RELOAD_DRIVER` plus toggles (`LOGGING`, `HDRPLUS`, `SDR10`, `CUSTOMEDID`, `HARDWARECURSOR`, …). There is no "add a monitor at W×H" command ([Driver.cpp `HandleClient`](https://github.com/VirtualDrivers/Virtual-Display-Driver/blob/master/Virtual%20Display%20Driver%20(HDR)/MttVDD/Driver.cpp)).
  - Every Area resize would mean rewriting the XML and reloading the whole adapter.
  - The README warns about black screens after GPU-driver updates, and the last release was 2025-07-23 ([#497](https://github.com/VirtualDrivers/Virtual-Display-Driver/issues/497) asks why there has been none since).
  - Its signing is the one real advantage.
- **parsec-vdd:**
  - Add/remove carry no mode. Modes are the presets plus up to 5 `HKLM\SOFTWARE\Parsec\vdd` entries, read before connecting (README "Known Limitations 2").
  - It needs a ping every <100 ms (`VddUpdate` doc comment in [parsec-vdd.h](https://github.com/nomi-san/parsec-vdd/blob/main/core/parsec-vdd.h)).
  - Its license blocks bundling.
- **IddSampleDriver / RustDeskIddDriver:** unsigned, with fixed modes (VDD and parsec-vdd comparison tables).
- **Own driver from scratch:** it would rebuild SudoVDA. Forking SudoVDA gets the same result.

## Resizing the Virtual monitor mid-session

- With stock SudoVDA: `IOCTL_REMOVE_VIRTUAL_DISPLAY(guid)`, then `IOCTL_ADD_VIRTUAL_DISPLAY(guid, newW, newH, 60)`, then `ChangeDisplaySettingsExW`. To Windows this is an unplug followed by a hotplug.
- **[test]** Does Discord's Screen share survive the monitor disappearing and coming back? Expect it to end or freeze; Discord would then have to be pointed at the screen again.
- The proper fix is a driver change. IddCx provides `IddCxMonitorUpdateModes` (Windows 10+, the SDR path) and `IddCxMonitorUpdateModes2` (IddCx 1.10, Windows 11 22H2+, required if the driver reports HDR). These let a driver replace an existing monitor's mode list ([UpdateModes](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/iddcx/nf-iddcx-iddcxmonitorupdatemodes), [UpdateModes2](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/iddcx/nf-iddcx-iddcxmonitorupdatemodes2)).
- A cropcast fork would add something like `IOCTL_UPDATE_MODE {GUID, W, H, Hz}`. It would store the new preferred mode in the monitor context and call `IddCxMonitorUpdateModes2` (SudoVDA reports HDR on 24H2), with `SudoVDAMonitorQueryModes` reporting the new size. The app would then call `ChangeDisplaySettingsExW`.
- **[test]** Even a mode change on the same monitor may restart Discord's capture. Windows.Graphics.Capture and DXGI duplication both see the output resize (see the ACCESS_LOST note below).

## Keeping the Virtual monitor out of the way

Windows has no "hidden" monitor, so these are design inputs for the lifecycle ticket, not settled answers:

- **Placement:** put the Virtual monitor so it touches the real monitor only at a corner, for example top-left at (5120, 1440). The cursor then can only reach it through that single corner point. Apollo has positioning code for this idea (`rearrangeVirtualDisplayForLowerRight` in [virtual_display.cpp](https://github.com/ClassicOldSong/Apollo/blob/master/src/platform/windows/virtual_display.cpp)). **[test]** Whether Windows keeps a corner-only adjacency or snaps the monitor to an edge.
- **Never primary:** do not pass `CDS_SET_PRIMARY`. New windows open on the primary monitor, the 5120×1440 panel.
- **Cursor:** `ClipCursor` to the real monitor works, but Windows resets the clip on focus and desktop changes, so it would need re-applying, for example from a `WM_ACTIVATEAPP` or `SetWinEventHook` handler. A low-level mouse hook (`WH_MOUSE_LL`) is the stronger option. Both are left to the lifecycle ticket. **[test]**
- **Window drift:** Win+Shift+Arrow and "move to other monitor" can still send windows there. Accept this for v1.

## Capture compatibility

- An IddCx monitor is a normal WDDM output: Sunshine and Apollo capture it with DXGI Desktop Duplication. I found no Discord-specific reports in the SudoVDA, Apollo or VDD trackers.
- Relevant open issues:
  - [VDD #399](https://github.com/VirtualDrivers/Virtual-Display-Driver/issues/399): `IDXGIOutputDuplication` gets stuck returning `DXGI_ERROR_ACCESS_LOST` after a resolution switch on an IDD monitor and does not recover by recreating it. A commenter: "When I switch the resolution from 1K to 2K or 4K, the capture program always encounters a DXGI_ERROR_ACCESS_LOST error". **This is exactly the resize scenario.** If Discord uses Desktop Duplication on Lumi's setup, resizes may break the stream **[test]**.
  - [Apollo #1376](https://github.com/ClassicOldSong/Apollo/issues/1376): "Black screen at 5120x1440 ultra-wide". This concerns streaming a 5120×1440 *virtual* display. It is not cropcast's case, but worth knowing.
- What Discord itself captures with (WGC vs. Desktop Duplication vs. its native hook), and whether it shows the Virtual monitor in the Screen picker, can only be checked hands-on **[test]**.

## Hands-on tests that settle the open points

1. Install Apollo 0.4.6's SudoVDA package with Secure Boot on. Confirm it loads without test-signing.
2. Add a monitor at 2381×1307@60 from a non-elevated process. Confirm Windows applies that exact mode.
3. Share that monitor in Discord (Screen, with audio), then resize by remove + re-add. Does the share survive, freeze or end?
4. Change the mode on the same monitor (fork prototype or `ChangeDisplaySettingsExW` between exposed modes). Does Discord follow?
5. Place the monitor at the corner. Does Windows keep it there, and can the cursor cross?
6. Kill cropcast. Does the monitor vanish within the watchdog time?
