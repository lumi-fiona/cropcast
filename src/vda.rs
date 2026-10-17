//! SudoVDA control: open the device, add/remove the Virtual monitor, set its mode and position.
//! IOCTL layout from SudoVDA Common/Include/sudovda-ioctl.h (IOCTL_UPDATE_MODE from cropcast's
//! fork, lumi-fiona/SudoVDA), call sequence from Apollo's third-party/sudovda/sudovda.h +
//! virtual_display.cpp.

use std::ffi::c_void;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{JoinHandle, sleep};
use std::time::{Duration, Instant};

use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::{Error, GUID, PCWSTR, Result};

const INTERFACE_GUID: GUID = GUID::from_u128(0xe5bcc234_1e0c_418a_a0d4_ef8b7501414d);
/// One fixed GUID: the EDID is derived from it, so Windows sees the same monitor every time.
const MONITOR_GUID: GUID = GUID::from_u128(0x3c0c7a5f_6e1b_4d2a_9c41_c40fca57e006);

/// CTL_CODE(FILE_DEVICE_UNKNOWN, f, METHOD_BUFFERED, FILE_ANY_ACCESS)
const fn ctl(f: u32) -> u32 {
    (0x22 << 16) | (f << 2)
}
const IOCTL_ADD: u32 = ctl(0x800);
const IOCTL_REMOVE: u32 = ctl(0x801);
const IOCTL_UPDATE_MODE: u32 = ctl(0x804);
const IOCTL_PING: u32 = ctl(0x888);
const IOCTL_GET_PROTOCOL_VERSION: u32 = ctl(0x8FF);

#[repr(C)]
struct AddParams {
    width: u32,
    height: u32,
    refresh_rate: u32,
    monitor_guid: GUID,
    device_name: [u8; 14],
    serial_number: [u8; 14],
}

#[repr(C)]
struct UpdateModeParams {
    monitor_guid: GUID,
    width: u32,
    height: u32,
    refresh_rate: u32,
}
const _: () = assert!(IOCTL_UPDATE_MODE == 0x222010 && size_of::<UpdateModeParams>() == 28);

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct AddOut {
    adapter_luid: LUID,
    target_id: u32,
}

#[repr(C)]
#[derive(Default)]
struct ProtocolVersion {
    major: u8,
    minor: u8,
    incremental: u8,
    test_build: u8,
}

pub(crate) fn fail(msg: impl Into<String>) -> Error {
    Error::new(E_FAIL, msg.into())
}

fn cstr14(s: &str) -> [u8; 14] {
    let mut b = [0u8; 14];
    b[..s.len().min(13)].copy_from_slice(&s.as_bytes()[..s.len().min(13)]);
    b
}

struct Vda(HANDLE);

// The handle is only used for synchronous DeviceIoControl, which is thread-safe.
unsafe impl Send for Vda {}

impl Drop for Vda {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl Vda {
    fn open() -> Result<Vda> {
        unsafe {
            let set = SetupDiGetClassDevsW(
                Some(&INTERFACE_GUID),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )?;
            let mut found = Err(fail(
                "The SudoVDA driver isn't installed or isn't running, so there's no Virtual monitor to share.",
            ));
            let mut data = SP_DEVICE_INTERFACE_DATA {
                cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            let mut i = 0;
            while found.is_err()
                && SetupDiEnumDeviceInterfaces(set, None, &INTERFACE_GUID, i, &mut data).is_ok()
            {
                i += 1;
                let mut size = 0u32;
                let _ = SetupDiGetDeviceInterfaceDetailW(set, &data, None, 0, Some(&mut size), None);
                let mut buf = vec![0u64; (size as usize).div_ceil(8)];
                let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
                (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                if SetupDiGetDeviceInterfaceDetailW(set, &data, Some(detail), size, None, None).is_err() {
                    continue;
                }
                if let Ok(h) = CreateFileW(
                    PCWSTR((*detail).DevicePath.as_ptr()),
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                ) {
                    found = Ok(Vda(h));
                }
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
            found
        }
    }

    fn ioctl<I, O>(&self, code: u32, input: Option<&I>, output: Option<&mut O>) -> Result<()> {
        let mut returned = 0u32;
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                input.map(|p| p as *const I as *const c_void),
                input.map_or(0, |_| size_of::<I>() as u32),
                output.map(|p| p as *mut O as *mut c_void),
                size_of::<O>() as u32,
                Some(&mut returned),
                None,
            )
        }
    }

    /// Refuses a major mismatch or an older minor (Apollo's rule): the structs above are 0.3.x,
    /// which only cropcast's fork speaks, so a stock 0.2.x SudoVDA gets the Driver installed over it.
    fn check_version(&self) -> Result<()> {
        let mut v = ProtocolVersion::default();
        self.ioctl::<(), _>(IOCTL_GET_PROTOCOL_VERSION, None, Some(&mut v))?;
        if v.major != 0 || v.minor < 3 {
            return Err(fail(format!(
                "This SudoVDA speaks protocol {}.{}.{}; cropcast needs 0.3.x.",
                v.major, v.minor, v.incremental
            )));
        }
        Ok(())
    }
}

/// Whether a SudoVDA is running that speaks cropcast's protocol.
pub fn ready() -> bool {
    Vda::open().and_then(|v| v.check_version()).is_ok()
}

/// Pings the driver every second on its own handle and thread, so the Virtual monitor
/// outlives a busy UI but not cropcast: if the process dies, the 3 s watchdog removes it.
/// Dropping the sender stops it; joining closes its handle, which removing the Driver needs.
fn start_watchdog() -> Result<(Sender<()>, JoinHandle<()>)> {
    let vda = Vda::open()?;
    let (stop, stopped) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        loop {
            let _ = vda.ioctl::<(), ()>(IOCTL_PING, None, None);
            if stopped.recv_timeout(Duration::from_secs(1)) != Err(RecvTimeoutError::Timeout) {
                break;
            }
        }
    });
    Ok((stop, thread))
}

/// `\\.\DISPLAYn` (NUL-terminated) of the active path whose target is {luid, target_id}.
fn gdi_name(out: AddOut) -> Option<Vec<u16>> {
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return None;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            != ERROR_SUCCESS
        {
            return None;
        }
        let p = paths[..np as usize]
            .iter()
            .find(|p| p.targetInfo.adapterId == out.adapter_luid && p.targetInfo.id == out.target_id)?;
        let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        name.header.adapterId = p.sourceInfo.adapterId;
        name.header.id = p.sourceInfo.id;
        if DisplayConfigGetDeviceInfo(&mut name.header) != 0 {
            return None;
        }
        let len = name.viewGdiDeviceName.iter().position(|&c| c == 0).unwrap_or(32);
        let mut v = name.viewGdiDeviceName[..len].to_vec();
        v.push(0);
        Some(v)
    }
}

/// The Virtual monitor while it exists. Dropping it removes the monitor.
pub struct Display {
    vda: Vda,
    watchdog: Option<(Sender<()>, JoinHandle<()>)>,
    /// The adapter and target the Driver gave it.
    out: AddOut,
    /// `\\.\DISPLAYn`, NUL-terminated.
    pub name: Vec<u16>,
    /// Where Windows put it, in desktop pixels.
    pub rect: RECT,
}

impl Display {
    /// Adds a `w`×`h`@60 monitor and puts its top-left at `anchor`.
    pub fn add(w: u32, h: u32, anchor: POINT) -> Result<Display> {
        let vda = Vda::open()?;
        vda.check_version()?;
        let watchdog = Some(start_watchdog()?);

        let p = AddParams {
            width: w,
            height: h,
            refresh_rate: 60,
            monitor_guid: MONITOR_GUID,
            device_name: cstr14("cropcast"),
            serial_number: cstr14("cropcast"),
        };
        let mut out = AddOut::default();
        vda.ioctl(IOCTL_ADD, Some(&p), Some(&mut out))?;
        // From here on, dropping `display` removes the monitor again.
        let mut display = Display { vda, watchdog, out, name: Vec::new(), rect: RECT::default() };

        // The path shows up in QueryDisplayConfig a little after the IOCTL returns.
        let t = Instant::now();
        let mut delay = 20;
        display.name = loop {
            if let Some(n) = gdi_name(out) {
                break n;
            }
            if t.elapsed() > Duration::from_secs(5) {
                return Err(fail("The Virtual monitor was added but Windows never brought it up."));
            }
            sleep(Duration::from_millis(delay));
            delay = (delay * 2).min(320);
        };

        display.rect = set_mode(&display.name, w, h, anchor)?;
        Ok(display)
    }

    /// Changes the live monitor to `w`×`h`@60 with its top-left at `anchor`, so Windows (and Discord)
    /// see the same monitor. Fails if Windows won't take that size.
    pub fn update_mode(&mut self, w: u32, h: u32, anchor: POINT) -> Result<()> {
        self.offer_mode(w, h)?;
        self.rect = set_mode(&self.name, w, h, anchor)?;
        let r = self.rect;
        if (r.right - r.left, r.bottom - r.top) != (w as i32, h as i32) {
            return Err(fail(format!("Windows put the Virtual monitor at {}×{}, not {w}×{h}.", r.right - r.left, r.bottom - r.top)));
        }
        Ok(())
    }

    /// Whether Windows still has this monitor up, and not as the primary, which it becomes when the
    /// real monitor goes away. It's gone when the Driver is removed or its watchdog timed out.
    pub fn alive(&self) -> bool {
        let primary = unsafe {
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let _ = GetMonitorInfoW(monitor, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO);
            mi.szDevice
        };
        // Both NUL-terminated, so \\.\DISPLAY1 doesn't match \\.\DISPLAY12.
        gdi_name(self.out).is_some_and(|n| n == self.name) && !primary.starts_with(&self.name)
    }

    /// Makes `w`×`h`@60 the Driver's preferred mode for the monitor and pushes its new mode list to
    /// Windows. Doesn't switch to it: `update_mode` does.
    pub fn offer_mode(&self, w: u32, h: u32) -> Result<()> {
        let p = UpdateModeParams { monitor_guid: MONITOR_GUID, width: w, height: h, refresh_rate: 60 };
        self.vda.ioctl::<_, ()>(IOCTL_UPDATE_MODE, Some(&p), None)
    }
}

/// Switches `name` to `w`×`h`@60 with its top-left at `pos`, and returns where Windows put it.
fn set_mode(name: &[u16], w: u32, h: u32, pos: POINT) -> Result<RECT> {
    unsafe {
        let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
        dm.dmPelsWidth = w;
        dm.dmPelsHeight = h;
        dm.dmDisplayFrequency = 60;
        dm.Anonymous1.Anonymous2.dmPosition = POINTL { x: pos.x, y: pos.y };
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_POSITION;
        // Never CDS_SET_PRIMARY.
        let r = ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_UPDATEREGISTRY, None);
        if r != DISP_CHANGE_SUCCESSFUL {
            return Err(fail(format!("Windows refused {w}×{h} for the Virtual monitor (code {}).", r.0)));
        }
        // Read back what Windows actually applied: that's where the presenter has to go.
        let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
        if !EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
            return Err(fail("Couldn't read the Virtual monitor's mode back."));
        }
        let pos = dm.Anonymous1.Anonymous2.dmPosition;
        Ok(RECT { left: pos.x, top: pos.y, right: pos.x + dm.dmPelsWidth as i32, bottom: pos.y + dm.dmPelsHeight as i32 })
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        // If this fails, the watchdog still removes the monitor within its timeout.
        let _ = self.vda.ioctl::<_, ()>(IOCTL_REMOVE, Some(&MONITOR_GUID), None);
        if let Some((stop, thread)) = self.watchdog.take() {
            drop(stop);
            let _ = thread.join();
        }
    }
}
