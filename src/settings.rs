//! The Area across restarts: one "x y w h" line in %APPDATA%\cropcast\settings. Not next to the exe,
//! which may sit in a read-only or Downloads folder.

use std::path::PathBuf;

use crate::edit::MIN;
use crate::mirror::Area;
use crate::virtual_monitor::real_monitor;

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("cropcast").join("settings"))
}

/// The saved Area, if there is one that still fits the real monitor.
pub fn load() -> Option<Area> {
    let r = real_monitor();
    parse(&std::fs::read_to_string(path()?).ok()?, (r.right - r.left, r.bottom - r.top))
}

// ponytail: a plain overwrite; a write torn by a crash fails `parse` and the Area falls back to the default.
pub fn save(a: Area) -> std::io::Result<()> {
    let p = path().ok_or(std::io::ErrorKind::NotFound)?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(p, format!("{} {} {} {}\n", a.x, a.y, a.w, a.h))
}

/// An "x y w h" line as an Area on a `bounds` monitor: even, at least MIN, no bigger than the
/// monitor, and moved back onto it. None for anything else.
pub fn parse(s: &str, bounds: (i32, i32)) -> Option<Area> {
    let (mw, mh) = bounds;
    let n: Vec<i32> = s.split_whitespace().map(|t| t.parse().ok()).collect::<Option<_>>()?;
    let &[x, y, w, h] = n.as_slice() else { return None };
    if w % 2 != 0 || h % 2 != 0 || !(MIN..=mw).contains(&w) || !(MIN..=mh).contains(&h) {
        return None;
    }
    Some(Area { x: x.clamp(0, mw - w), y: y.clamp(0, mh - h), w: w as u32, h: h as u32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: (i32, i32) = (5120, 1440);

    #[test]
    fn a_saved_line_round_trips() {
        assert_eq!(parse("1280 0 2560 1440\n", MON), Some(Area { x: 1280, y: 0, w: 2560, h: 1440 }));
        assert_eq!(parse("  10   20 1000 800 ", MON), Some(Area { x: 10, y: 20, w: 1000, h: 800 }));
    }

    #[test]
    fn an_area_off_the_monitor_is_moved_back_on() {
        assert_eq!(parse("4000 900 2560 1440", MON), Some(Area { x: 2560, y: 0, w: 2560, h: 1440 }));
        assert_eq!(parse("-50 -3 1000 800", MON), Some(Area { x: 0, y: 0, w: 1000, h: 800 }));
    }

    #[test]
    fn a_bad_line_is_rejected() {
        for s in [
            "",
            "garbage",
            "1 2 3",
            "0 0 2560 1440 7",
            "0 0 2561 1440",            // odd
            "0 0 2560 1439",            // odd
            "0 0 358 1440",             // under MIN
            "0 0 2560 -1440",           // negative
            "0 0 5122 1440",            // wider than the monitor
            "0 0 2560 1442",            // taller than the monitor
            "0 0 2560.0 1440",          // not an integer
            "0 0 99999999999 1440",     // overflows i32
        ] {
            assert_eq!(parse(s, MON), None, "{s:?}");
        }
    }

    #[test]
    fn a_monitor_smaller_than_the_saved_area_falls_back() {
        assert_eq!(parse("0 0 2560 1440", (1920, 1080)), None);
    }
}
