//! Installing and removing the Driver (ADR 0002). The tray instance relaunches cropcast elevated
//! with a flag, and that copy does the admin work: certificates through CryptoAPI, the root device
//! node through SetupAPI + newdev (what nefcon does), driver-store chores through pnputil.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Foundation::*;
use windows::Win32::Security::Cryptography::*;
use windows::Win32::System::Threading::{CREATE_NO_WINDOW, GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    IsUserAnAdmin, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{BOOL, Error, HRESULT, HSTRING, PCWSTR, Result, w};

use crate::vda::{self, fail};

const INSTALL_FLAG: &str = "--install-driver";
const UNINSTALL_FLAG: &str = "--uninstall";

const HARDWARE_ID: PCWSTR = w!(r"root\sudomaker\sudovda");
/// Signer names as pnputil reports them, which are also the certificates' subjects.
const SIGNER: &str = "cropcast driver";
const APOLLO_SIGNER: &str = "sudovda@su.mk";
const STORES: [PCWSTR; 2] = [w!("Root"), w!("TrustedPublisher")];

struct Package {
    files: [(&'static str, &'static [u8]); 3],
    /// Trusted straight from memory, so nothing can swap it in the temp folder first.
    cer: &'static [u8],
}

/// The CI `driver` job's artifact, unpacked into driver/ before building with `--features driver`.
#[cfg(feature = "driver")]
const PACKAGE: Option<Package> = Some(Package {
    files: [
        ("SudoVDA.inf", include_bytes!("../driver/SudoVDA.inf")),
        ("SudoVDA.dll", include_bytes!("../driver/SudoVDA.dll")),
        ("sudovda.cat", include_bytes!("../driver/sudovda.cat")),
    ],
    cer: include_bytes!("../driver/cropcast-driver.cer"),
});
#[cfg(not(feature = "driver"))]
const PACKAGE: Option<Package> = None;

/// Runs `--install-driver` (the elevated half of `install`) or `--uninstall`, and returns the
/// exit code: 0 or the failure's HRESULT. None for a normal start.
pub fn run_flag() -> Option<i32> {
    let r = match std::env::args().nth(1)?.as_str() {
        INSTALL_FLAG => install_here(),
        UNINSTALL_FLAG if unsafe { IsUserAnAdmin() }.as_bool() => uninstall_here(),
        UNINSTALL_FLAG => elevated(UNINSTALL_FLAG),
        _ => return None,
    };
    Some(r.map_or_else(|e| e.code().0, |()| 0))
}

/// Whether turning the Virtual monitor on has to install the Driver first: no SudoVDA speaks
/// cropcast's protocol, or Apollo's SudoVDA is left over without the Apollo app.
pub fn needed() -> bool {
    !vda::ready() || (PACKAGE.is_some() && !apollo_app() && !oems(&drivers(), APOLLO_SIGNER).is_empty())
}

/// Installs the Driver through an elevated copy of cropcast and waits until it answers.
/// The error is a plain sentence for the tray.
pub fn install() -> Result<()> {
    if PACKAGE.is_none() {
        return Err(fail("This build of cropcast has no Driver inside, so it can't install one."));
    }
    match elevated(INSTALL_FLAG) {
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => {
            return Err(fail(
                "The Driver needs admin rights once and the prompt was declined. \
                 Turn the Virtual monitor on again to be asked again.",
            ));
        }
        Err(e) => {
            return Err(fail(format!("Installing the Driver failed, so cropcast undid it: {}", describe(&e))));
        }
        Ok(()) => {}
    }
    // The fresh device takes a moment to come up.
    let t = Instant::now();
    let mut delay = 20;
    while !vda::ready() {
        if t.elapsed() > Duration::from_secs(10) {
            return Err(fail("The Driver installed but didn't start. Restarting Windows usually fixes that."));
        }
        sleep(Duration::from_millis(delay));
        delay = (delay * 2).min(320);
    }
    Ok(())
}

/// Removes the Driver through an elevated copy of cropcast. The Virtual monitor must be off.
pub fn uninstall() -> Result<()> {
    elevated(UNINSTALL_FLAG).map_err(|e| {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            fail("Removing the Driver needs admin rights and the prompt was declined.")
        } else {
            fail(format!("Removing the Driver failed: {}", describe(&e)))
        }
    })
}

fn describe(e: &Error) -> String {
    format!("{} (0x{:08X})", e.message().trim(), e.code().0)
}

/// Runs this exe again as admin (the one UAC prompt) with `flag`, and waits for it.
// ponytail: blocks the tray's UI thread for the prompt and the install (seconds); nothing else needs it meanwhile.
fn elevated(flag: &str) -> Result<()> {
    let exe = HSTRING::from(std::env::current_exe().map_err(|e| fail(e.to_string()))?.as_os_str());
    let flag = HSTRING::from(flag);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(flag.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    let mut code = 0u32;
    unsafe {
        ShellExecuteExW(&mut info)?;
        WaitForSingleObject(info.hProcess, INFINITE);
        let r = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        r?;
    }
    HRESULT(code as i32).ok()
}

/// What `install_here` added, so a failure can take exactly that back out.
#[derive(Default)]
struct Added {
    certs: [bool; 2],
    node: bool,
}

/// The elevated half of `install`.
fn install_here() -> Result<()> {
    let Some(pkg) = PACKAGE else {
        return Err(E_NOTIMPL.into());
    };
    let dir = std::env::temp_dir().join(format!("cropcast-driver-{}", std::process::id()));
    let mut added = Added::default();
    let r = try_install(&pkg, &dir, &mut added);
    if r.is_err() {
        if added.node {
            remove_devices();
        }
        delete_packages(SIGNER);
        for (store, _) in STORES.iter().zip(added.certs).filter(|(_, a)| *a) {
            let _ = untrust(*store, SIGNER);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn try_install(pkg: &Package, dir: &Path, added: &mut Added) -> Result<()> {
    let io = |e: std::io::Error| fail(e.to_string());
    std::fs::create_dir_all(dir).map_err(io)?;
    for (name, bytes) in pkg.files {
        std::fs::write(dir.join(name), bytes).map_err(io)?;
    }
    for (store, a) in STORES.iter().zip(&mut added.certs) {
        *a = trust(*store, pkg.cer)?;
    }
    let inf = HSTRING::from(dir.join("SudoVDA.inf").as_os_str());
    // Updates an existing SudoVDA device (Apollo's or an older Driver) in place; otherwise
    // creates the root device node first.
    match update_driver(&inf) {
        Err(e) if e.code() == ERROR_NO_SUCH_DEVINST.to_hresult() => {
            create_node()?;
            added.node = true;
            update_driver(&inf)?;
        }
        r => r?,
    }
    // Our Driver is on the device now, so older packages aren't in use; pnputil refuses the one that is.
    delete_packages(SIGNER);
    if !apollo_app() {
        delete_packages(APOLLO_SIGNER);
        for store in STORES {
            let _ = untrust(store, APOLLO_SIGNER);
        }
    }
    Ok(())
}

/// The elevated half of `uninstall`: our devices, our packages, our certificate. Keeps going past
/// a failed step and returns the first error.
fn uninstall_here() -> Result<()> {
    let mut r = Ok(());
    remove_devices();
    for oem in oems(&drivers(), SIGNER) {
        r = r.and(pnputil(&["/delete-driver", &oem]));
    }
    for store in STORES {
        r = r.and(untrust(store, SIGNER));
    }
    r
}

fn update_driver(inf: &HSTRING) -> Result<()> {
    // FORCE: install over Apollo's SudoVDA even if Windows ranks it higher. Passing the reboot flag
    // keeps Windows from prompting for a restart itself; `install` reports a Driver that doesn't start.
    let mut reboot = BOOL(0);
    unsafe { UpdateDriverForPlugAndPlayDevicesW(None, HARDWARE_ID, inf, INSTALLFLAG_FORCE, Some(&mut reboot)) }
}

/// Creates the `root\sudomaker\sudovda` device node, like `nefconc --create-device-node`.
fn create_node() -> Result<()> {
    unsafe {
        let set = SetupDiCreateDeviceInfoList(Some(&GUID_DEVCLASS_DISPLAY), None)?;
        let mut data = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        // A multi-string: the ID, then an empty string.
        let id: Vec<u8> = HARDWARE_ID.as_wide().iter().chain(&[0, 0]).flat_map(|c| c.to_le_bytes()).collect();
        let r = SetupDiCreateDeviceInfoW(
            set,
            w!("Display"),
            &GUID_DEVCLASS_DISPLAY,
            None,
            None,
            DICD_GENERATE_ID,
            Some(&mut data),
        )
        .and_then(|()| SetupDiSetDeviceRegistryPropertyW(set, &mut data, SPDRP_HARDWAREID, Some(&id)))
        .and_then(|()| SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set, Some(&data)));
        let _ = SetupDiDestroyDeviceInfoList(set);
        r
    }
}

/// Removes the SudoVDA devices on our Driver, or on none (a failed install's node). A device on
/// Apollo's SudoVDA stays: the Apollo app may need it.
fn remove_devices() {
    let ours = oems(&drivers(), SIGNER);
    let xml = run("pnputil", &["/enum-devices", "/deviceid", r"root\sudomaker\sudovda", "/format", "xml"])
        .unwrap_or_default();
    for id in devices_on(&xml, &ours) {
        let _ = pnputil(&["/remove-device", &id]);
    }
}

/// Deletes the SudoVDA packages `signer` signed; pnputil refuses any a device still uses.
fn delete_packages(signer: &str) {
    for oem in oems(&drivers(), signer) {
        let _ = pnputil(&["/delete-driver", &oem]);
    }
}

/// The Apollo app runs as ApolloService; without it nothing else uses Apollo's SudoVDA.
fn apollo_app() -> bool {
    run("sc.exe", &["query", "ApolloService"]).is_ok()
}

fn drivers() -> String {
    run("pnputil", &["/enum-drivers", "/class", "Display", "/format", "xml"]).unwrap_or_default()
}

fn pnputil(args: &[&str]) -> Result<()> {
    run("pnputil", args).map(drop)
}

/// Runs a console tool without flashing a window. A non-zero exit is its Win32 error code;
/// 3010 (reboot required) counts as done.
fn run(exe: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()
        .map_err(|e| fail(format!("{exe}: {e}")))?;
    match out.status.code() {
        Some(0 | 3010) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        c => Err(HRESULT::from_win32(c.unwrap_or(1) as u32).into()),
    }
}

/// The SudoVDA packages in `pnputil /enum-drivers /format xml` that `signer` signed, as oemN.inf.
// ponytail: string scanning, not an XML parser; pnputil's element names don't vary and these values hold no entities.
fn oems(xml: &str, signer: &str) -> Vec<String> {
    elements(xml, "Driver")
        .into_iter()
        .filter(|(_, body)| field(body, "OriginalName").is_some_and(|n| n.eq_ignore_ascii_case("sudovda.inf")))
        .filter(|(_, body)| field(body, "SignerName") == Some(signer))
        .map(|(oem, _)| oem.to_owned())
        .collect()
}

/// Instance IDs in `pnputil /enum-devices /format xml` whose driver is one of `oems`, or none.
fn devices_on(xml: &str, oems: &[String]) -> Vec<String> {
    elements(xml, "Device")
        .into_iter()
        .filter(|(_, body)| field(body, "DriverName").is_none_or(|d| oems.iter().any(|o| o.eq_ignore_ascii_case(d))))
        .map(|(id, _)| id.to_owned())
        .collect()
}

/// `<tag Attr="value">body</tag>` elements as (value, body).
fn elements<'a>(xml: &'a str, tag: &str) -> Vec<(&'a str, &'a str)> {
    let close = format!("</{tag}>");
    xml.split(&format!("<{tag} "))
        .skip(1)
        .filter_map(|s| {
            let body = s.split(&close).next()?;
            Some((body.split('"').nth(1)?, body))
        })
        .collect()
}

/// The text of `<tag>…</tag>` in `body`.
fn field<'a>(body: &'a str, tag: &str) -> Option<&'a str> {
    body.split(&format!("<{tag}>")).nth(1)?.split('<').next()
}

/// Adds the certificate to a LocalMachine store; false if it was already there.
fn trust(store: PCWSTR, cer: &[u8]) -> Result<bool> {
    let s = open_store(store)?;
    let r = unsafe { CertAddEncodedCertificateToStore(Some(s), X509_ASN_ENCODING, cer, CERT_STORE_ADD_NEW, None) };
    unsafe {
        let _ = CertCloseStore(Some(s), 0);
    }
    match r {
        Ok(()) => Ok(true),
        Err(e) if e.code() == CRYPT_E_EXISTS => Ok(false),
        Err(e) => Err(e),
    }
}

/// Deletes every certificate in a LocalMachine store whose subject contains `subject`.
fn untrust(store: PCWSTR, subject: &str) -> Result<()> {
    let s = open_store(store)?;
    let subject = HSTRING::from(subject);
    let mut r = Ok(());
    unsafe {
        while r.is_ok() {
            let c = CertFindCertificateInStore(
                s,
                X509_ASN_ENCODING,
                0,
                CERT_FIND_SUBJECT_STR,
                Some(subject.as_ptr().cast()),
                None,
            );
            if c.is_null() {
                break;
            }
            r = CertDeleteCertificateFromStore(c); // frees `c`, even on failure
        }
        let _ = CertCloseStore(Some(s), 0);
    }
    r
}

fn open_store(name: PCWSTR) -> Result<HCERTSTORE> {
    unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            CERT_QUERY_ENCODING_TYPE(0),
            None,
            CERT_OPEN_STORE_FLAGS(CERT_SYSTEM_STORE_LOCAL_MACHINE),
            Some(name.as_ptr().cast()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRIVERS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<PnpUtil Version="10.0.26100" Command="/enum-drivers /class Display /format xml">
    <Driver DriverName="oem76.inf">
        <OriginalName>sudovda.inf</OriginalName>
        <ProviderName>SudoMaker</ProviderName>
        <DriverVersion>07/14/2025 1.10.9.289</DriverVersion>
        <SignerName>sudovda@su.mk</SignerName>
    </Driver>
    <Driver DriverName="oem8.inf">
        <OriginalName>tvvirtualmonitordriver.inf</OriginalName>
        <SignerName>Microsoft Windows Hardware Compatibility Publisher</SignerName>
    </Driver>
    <Driver DriverName="oem81.inf">
        <OriginalName>SudoVDA.inf</OriginalName>
        <SignerName>cropcast driver</SignerName>
    </Driver>
    <Driver DriverName="oem82.inf">
        <OriginalName>sudovda.inf</OriginalName>
        <SignerName>cropcast driver</SignerName>
    </Driver>
</PnpUtil>"#;

    #[test]
    fn oems_pick_sudovda_packages_by_signer() {
        assert_eq!(oems(DRIVERS, SIGNER), ["oem81.inf", "oem82.inf"]);
        assert_eq!(oems(DRIVERS, APOLLO_SIGNER), ["oem76.inf"]);
        assert!(oems(DRIVERS, "Microsoft Windows Hardware Compatibility Publisher").is_empty());
        assert!(oems("", SIGNER).is_empty());
    }

    #[test]
    fn devices_on_skips_apollos() {
        let xml = r#"<PnpUtil>
    <Device InstanceId="ROOT\DISPLAY\0000">
        <Status>Started</Status>
        <DriverName>oem76.inf</DriverName>
    </Device>
    <Device InstanceId="ROOT\DISPLAY\0001">
        <DriverName>OEM81.inf</DriverName>
    </Device>
    <Device InstanceId="ROOT\DISPLAY\0002">
        <Status>Problem</Status>
    </Device>
</PnpUtil>"#;
        let ours = oems(DRIVERS, SIGNER);
        assert_eq!(devices_on(xml, &ours), [r"ROOT\DISPLAY\0001", r"ROOT\DISPLAY\0002"]);
        assert!(devices_on("No devices were found on the system.", &ours).is_empty());
    }
}
