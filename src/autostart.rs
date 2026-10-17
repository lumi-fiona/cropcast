//! Start with Windows: the HKCU Run value `cropcast`, holding this exe's quoted path. The registry is
//! the only record of it.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, Result, w};

const RUN: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const NAME: windows::core::PCWSTR = w!("cropcast");

fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
}

/// Whether a Run value starts `exe`, quoted or not, in any case.
pub fn points_at(value: &str, exe: &str) -> bool {
    !exe.is_empty() && value.trim().trim_matches('"').eq_ignore_ascii_case(exe)
}

/// True when the Run value exists and starts this exe.
pub fn enabled() -> bool {
    // ponytail: a fixed buffer; a value longer than 2048 chars isn't one of ours.
    let mut buf = [0u16; 2048];
    let mut len = size_of_val(&buf) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    let value = String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]);
    r.is_ok() && points_at(&value, &exe())
}

pub fn set(on: bool) -> Result<()> {
    let r: WIN32_ERROR = unsafe {
        if on {
            let value = HSTRING::from(format!("\"{}\"", exe()));
            // REG_SZ data includes its terminating NUL.
            RegSetKeyValueW(HKEY_CURRENT_USER, RUN, NAME, REG_SZ.0, Some(value.as_ptr().cast()), (value.len() as u32 + 1) * 2)
        } else {
            match RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) {
                ERROR_FILE_NOT_FOUND => WIN32_ERROR(0),
                r => r,
            }
        }
    };
    r.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_value_points_at_this_exe_quoted_or_not() {
        let exe = r"C:\Tools\cropcast.exe";
        assert!(points_at(r#""C:\Tools\cropcast.exe""#, exe));
        assert!(points_at(r"c:\tools\CROPCAST.exe", exe));
        assert!(!points_at(r#""D:\Old\cropcast.exe""#, exe));
        assert!(!points_at("", exe));
        assert!(!points_at("", ""));
    }

    /// Writes the real HKCU Run key, so only on request: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn set_and_unset_the_run_value() {
        let before = enabled();
        set(true).unwrap();
        assert!(enabled());
        set(false).unwrap();
        assert!(!enabled());
        set(false).unwrap(); // already gone is fine
        set(before).unwrap();
    }
}
