//! SudoVDA control: open the device, add/remove the Virtual monitor, find its GDI name,
//! set its mode and position. IOCTL layout from SudoVDA Common/Include/sudovda-ioctl.h,
//! call sequence from Apollo third-party/sudovda/sudovda.h + virtual_display.cpp.

use std::ffi::c_void;
use std::mem::size_of;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::{Error, GUID, PCWSTR, Result};

pub const INTERFACE_GUID: GUID = GUID::from_u128(0xe5bcc234_1e0c_418a_a0d4_ef8b7501414d);
/// One fixed GUID: the EDID is derived from it, so Windows sees the same monitor every time.
pub const MONITOR_GUID: GUID = GUID::from_u128(0x3c0c7a5f_6e1b_4d2a_9c41_c40fca57e006);

/// CTL_CODE(FILE_DEVICE_UNKNOWN, f, METHOD_BUFFERED, FILE_ANY_ACCESS)
const fn ctl(f: u32) -> u32 {
    (0x22 << 16) | (f << 2)
}
const IOCTL_ADD: u32 = ctl(0x800);
const IOCTL_REMOVE: u32 = ctl(0x801);
const IOCTL_GET_WATCHDOG: u32 = ctl(0x803);
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
#[derive(Default, Clone, Copy)]
pub struct AddOut {
    pub adapter_luid: LUID,
    pub target_id: u32,
}

#[repr(C)]
#[derive(Default)]
struct ProtocolVersion {
    major: u8,
    minor: u8,
    incremental: u8,
    test_build: u8,
}

#[repr(C)]
#[derive(Default)]
struct Watchdog {
    timeout: u32,
    countdown: u32,
}

fn fail(msg: impl Into<String>) -> Error {
    Error::new(E_FAIL, msg.into())
}

fn cstr14(s: &str) -> [u8; 14] {
    let mut b = [0u8; 14];
    b[..s.len().min(13)].copy_from_slice(&s.as_bytes()[..s.len().min(13)]);
    b
}

pub struct Vda(HANDLE);

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
    /// Opens the SudoVDA device interface, retrying 20..320 ms like Apollo does right after install.
    pub fn open() -> Result<Vda> {
        let mut delay = 20;
        loop {
            if let Some(v) = Self::try_open()? {
                return Ok(v);
            }
            if delay > 320 {
                return Err(fail(format!(
                    "SudoVDA device interface {{{INTERFACE_GUID:?}}} not found: the driver is not installed or not running.\n\
                     Install it with spike\\driver\\install.ps1 (elevated), or run with --preview to test without it."
                )));
            }
            sleep(Duration::from_millis(delay));
            delay *= 2;
        }
    }

    fn try_open() -> Result<Option<Vda>> {
        unsafe {
            let set = SetupDiGetClassDevsW(
                Some(&INTERFACE_GUID),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )?;
            let mut found = None;
            let mut data = SP_DEVICE_INTERFACE_DATA {
                cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            let mut i = 0;
            while found.is_none()
                && SetupDiEnumDeviceInterfaces(set, None, &INTERFACE_GUID, i, &mut data).is_ok()
            {
                i += 1;
                let mut size = 0u32;
                let _ =
                    SetupDiGetDeviceInterfaceDetailW(set, &data, None, 0, Some(&mut size), None);
                let mut buf = vec![0u64; (size as usize).div_ceil(8)];
                let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
                (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                if SetupDiGetDeviceInterfaceDetailW(set, &data, Some(detail), size, None, None)
                    .is_err()
                {
                    continue;
                }
                let path = PCWSTR((*detail).DevicePath.as_ptr());
                match CreateFileW(
                    path,
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                ) {
                    Ok(h) => found = Some(Vda(h)),
                    Err(e) => eprintln!("[vda] CreateFile({}) failed: {e}", path.display()),
                }
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
            Ok(found)
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

    /// Prints the protocol version and refuses a major mismatch or an older minor (Apollo's rule).
    pub fn check_version(&self) -> Result<()> {
        let mut v = ProtocolVersion::default();
        self.ioctl::<(), _>(IOCTL_GET_PROTOCOL_VERSION, None, Some(&mut v))?;
        println!(
            "[vda] protocol version {}.{}.{} (test build: {})",
            v.major,
            v.minor,
            v.incremental,
            v.test_build != 0
        );
        if v.major != 0 || v.minor < 2 {
            return Err(fail(format!(
                "unsupported SudoVDA protocol {}.{}.{}, need 0.2.x",
                v.major, v.minor, v.incremental
            )));
        }
        Ok(())
    }

    pub fn watchdog(&self) -> Result<(u32, u32)> {
        let mut w = Watchdog::default();
        self.ioctl::<(), _>(IOCTL_GET_WATCHDOG, None, Some(&mut w))?;
        Ok((w.timeout, w.countdown))
    }

    pub fn ping(&self) -> Result<()> {
        self.ioctl::<(), ()>(IOCTL_PING, None, None)
    }

    pub fn add(&self, w: u32, h: u32, hz: u32) -> Result<AddOut> {
        let p = AddParams {
            width: w,
            height: h,
            refresh_rate: hz,
            monitor_guid: MONITOR_GUID,
            device_name: cstr14("cropcast"),
            serial_number: cstr14("spike"),
        };
        let mut out = AddOut::default();
        self.ioctl(IOCTL_ADD, Some(&p), Some(&mut out))?;
        Ok(out)
    }

    pub fn remove(&self) -> Result<()> {
        self.ioctl::<_, ()>(IOCTL_REMOVE, Some(&MONITOR_GUID), None)
    }
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
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            None,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let p = paths[..np as usize].iter().find(|p| {
            p.targetInfo.adapterId == out.adapter_luid && p.targetInfo.id == out.target_id
        })?;
        let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        name.header.adapterId = p.sourceInfo.adapterId;
        name.header.id = p.sourceInfo.id;
        if DisplayConfigGetDeviceInfo(&mut name.header) != 0 {
            return None;
        }
        let len = name
            .viewGdiDeviceName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(32);
        let mut v = name.viewGdiDeviceName[..len].to_vec();
        v.push(0);
        Some(v)
    }
}

pub struct Mode {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub hz: u32,
}

fn current_mode(name: &[u16]) -> Option<Mode> {
    unsafe {
        let mut dm = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if !EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
            return None;
        }
        let pos = dm.Anonymous1.Anonymous2.dmPosition;
        Some(Mode {
            x: pos.x,
            y: pos.y,
            w: dm.dmPelsWidth,
            h: dm.dmPelsHeight,
            hz: dm.dmDisplayFrequency,
        })
    }
}

/// The Virtual monitor's lifecycle: on = added, named, mode set, positioned at `anchor`.
pub struct Display {
    pub vda: Vda,
    pub anchor: (i32, i32),
    pub size: (u32, u32),
    pub is_on: bool,
}

impl Display {
    /// Adds the monitor at `size`, 60 Hz, then sets its mode and puts its top-left at `anchor`.
    /// Returns the mode Windows actually applied.
    pub fn on(&mut self) -> Result<Mode> {
        let (w, h) = self.size;
        let t = Instant::now();
        let out = self.vda.add(w, h, 60)?;
        self.is_on = true;
        let t_add = t.elapsed();
        println!(
            "[vda] added {w}x{h}@60: adapter {:08x}:{:08x} target {} ({:.0} ms)",
            out.adapter_luid.HighPart,
            out.adapter_luid.LowPart,
            out.target_id,
            t_add.as_secs_f64() * 1e3
        );

        // Apollo's backoff, extended: the path shows up in QueryDisplayConfig a little after the IOCTL returns.
        let mut delay = 20;
        let mut tries = 0;
        let name = loop {
            tries += 1;
            if let Some(n) = gdi_name(out) {
                break n;
            }
            if t.elapsed() > Duration::from_secs(5) {
                return Err(fail(
                    "added the Virtual monitor but it never showed up in QueryDisplayConfig (5 s)",
                ));
            }
            sleep(Duration::from_millis(delay));
            delay = (delay * 2).min(320);
        };
        let t_name = t.elapsed();
        println!(
            "[vda] GDI name {} after {tries} tries ({:.0} ms)",
            String::from_utf16_lossy(&name[..name.len() - 1]),
            (t_name - t_add).as_secs_f64() * 1e3
        );

        let r = unsafe {
            let mut dm = DEVMODEW {
                dmSize: size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            dm.dmPelsWidth = w;
            dm.dmPelsHeight = h;
            dm.dmDisplayFrequency = 60;
            dm.Anonymous1.Anonymous2.dmPosition = POINTL {
                x: self.anchor.0,
                y: self.anchor.1,
            };
            dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_POSITION;
            // Never CDS_SET_PRIMARY.
            ChangeDisplaySettingsExW(
                PCWSTR(name.as_ptr()),
                Some(&dm),
                None,
                CDS_UPDATEREGISTRY,
                None,
            )
        };
        let t_mode = t.elapsed();
        if r != DISP_CHANGE_SUCCESSFUL {
            return Err(fail(format!(
                "ChangeDisplaySettingsExW({w}x{h}@60 at {:?}) returned {}",
                self.anchor, r.0
            )));
        }
        let m = current_mode(&name)
            .ok_or_else(|| fail("EnumDisplaySettingsW on the Virtual monitor failed"))?;
        println!(
            "[vda] mode set ({:.0} ms): asked {w}x{h}@60 at ({},{}), Windows applied {}x{}@{} at ({},{}){}",
            (t_mode - t_name).as_secs_f64() * 1e3,
            self.anchor.0,
            self.anchor.1,
            m.w,
            m.h,
            m.hz,
            m.x,
            m.y,
            if (m.w, m.h, m.x, m.y) == (w, h, self.anchor.0, self.anchor.1) {
                "  OK"
            } else {
                "  MISMATCH"
            }
        );
        Ok(m)
    }

    pub fn off(&mut self) -> Result<()> {
        if !self.is_on {
            return Ok(());
        }
        let t = Instant::now();
        self.vda.remove()?;
        self.is_on = false;
        println!(
            "[vda] removed the Virtual monitor ({:.0} ms)",
            t.elapsed().as_secs_f64() * 1e3
        );
        Ok(())
    }
}
