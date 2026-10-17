//! Edit mode: while Ctrl+Alt is held the Border shows, and dragging moves or resizes the Area.
//! The geometry is plain functions; `Session` drives the Border with them.

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LBUTTON, VK_MENU, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetSystemMetrics, SM_SWAPBUTTON};
use windows::core::Result;

use crate::border::{self, Border};
use crate::mirror::Area;
use crate::virtual_monitor::real_monitor;

/// How far into the Area an edge or corner can be grabbed, in px.
pub const EDGE: i32 = 12;
/// The smallest Area side, in px (even).
pub const MIN: i32 = 360;
/// A resize lands on a snap ratio within this fraction of it, and shows a guide to it within NEAR.
/// Dials to tune.
const SNAP: f64 = 0.015;
const NEAR: f64 = 0.07;
/// 21:9 means the 3440×1440 ultrawide (43:18), not true 21:9 (3360×1440).
const SNAP_RATIOS: [(i32, i32, &str); 2] = [(16, 9, "16:9"), (43, 18, "21:9")];

/// What a drag holds, per axis: -1 the left/top edge, 1 the right/bottom edge, 0 neither. (0, 0) moves.
pub type Grab = (i32, i32);

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

// Polled, not a WH_KEYBOARD_LL hook: a hook puts this thread in the path of every keystroke system-wide (and stalls them whenever the UI thread blocks, e.g. while a monitor is added); a 50 ms poll costs nothing per key.
pub fn hotkey_held() -> bool {
    held(VK_CONTROL) && held(VK_MENU)
}

/// GetAsyncKeyState reads physical buttons, so follow a left-handed swap.
fn button_held() -> bool {
    held(if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON })
}

/// The grab at (`x`, `y`) relative to a `w`×`h` Area's top-left; None outside it.
pub fn hit(w: i32, h: i32, x: i32, y: i32) -> Option<Grab> {
    let side = |p: i32, len: i32| if p < EDGE { -1 } else if p >= len - EDGE { 1 } else { 0 };
    ((0..w).contains(&x) && (0..h).contains(&y)).then(|| (side(x, w), side(y, h)))
}

/// `num / den` rounded to the nearest even number.
fn even(num: i32, den: i32) -> i32 {
    (num + den) / (2 * den) * 2
}

/// How far `w`×`h` is from the ratio `n`:`d`, as a fraction.
fn off(w: i32, h: i32, n: i32, d: i32) -> f64 {
    ((w * d) as f64 / (h * n) as f64 - 1.0).abs()
}

/// The Area after dragging `grab` of `from` by (`dx`, `dy`) on a `bounds` monitor: moved or resized,
/// kept on the monitor, at least MIN, snapped, and even. Near a snap ratio it also returns where the
/// Area would land on it.
pub fn drag(from: Area, grab: Grab, dx: i32, dy: i32, bounds: (i32, i32)) -> (Area, Option<Area>) {
    let (mw, mh) = bounds;
    let (gx, gy) = grab;
    if grab == (0, 0) {
        let x = (from.x + dx).min(mw - from.w as i32).max(0);
        let y = (from.y + dy).min(mh - from.h as i32).max(0);
        return (Area { x, y, ..from }, None);
    }
    // The grabbed edges follow the cursor; the others stay put.
    let (mut l, mut t, mut r, mut b) = (from.x, from.y, from.x + from.w as i32, from.y + from.h as i32);
    match gx {
        -1 => l = (l + dx).max(0).min(r - MIN),
        1 => r = (r + dx).min(mw).max(l + MIN),
        _ => {}
    }
    match gy {
        -1 => t = (t + dy).max(0).min(b - MIN),
        1 => b = (b + dy).min(mh).max(t + MIN),
        _ => {}
    }
    let max_w = if gx < 0 { r } else { mw - l };
    let max_h = if gy < 0 { b } else { mh - t };
    let (w, h) = ((r - l) & !1, (b - t) & !1);
    let place = |w: i32, h: i32| Area {
        x: if gx < 0 { r - w } else { l },
        y: if gy < 0 { b - h } else { t },
        w: w as u32,
        h: h as u32,
    };
    // The nearest ratio, reached along whichever dragged side moves less, if that still fits.
    let &(n, d, _) = SNAP_RATIOS.iter().min_by(|p, q| off(w, h, p.0, p.1).total_cmp(&off(w, h, q.0, q.1))).unwrap();
    let to = [(gx != 0).then(|| (even(h * n, d), h)), (gy != 0).then(|| (w, even(w * d, n)))]
        .into_iter()
        .flatten()
        .filter(|&(sw, sh)| (MIN..=max_w).contains(&sw) && (MIN..=max_h).contains(&sh))
        .min_by_key(|&(sw, sh)| (sw - w).abs() + (sh - h).abs());
    match to {
        Some((sw, sh)) if off(w, h, n, d) < SNAP => (place(sw, sh), None),
        Some((sw, sh)) if off(w, h, n, d) < NEAR => (place(w, h), Some(place(sw, sh))),
        _ => (place(w, h), None),
    }
}

/// The Border's readout: "2560 × 1440" and the ratio, named ("16:9") when the Area sits on a snap
/// ratio, else as a decimal ("2.29").
pub fn readout(w: u32, h: u32) -> (String, String, bool) {
    let (w, h) = (w as i32, h as i32);
    // A width snap rounds the width and a height snap the height, so either can land.
    let named = SNAP_RATIOS.iter().find(|&&(n, d, _)| w == even(h * n, d) || h == even(w * d, n));
    let ratio = named.map_or_else(|| format!("{:.2}", w as f64 / h as f64), |r| r.2.to_string());
    (format!("{w} × {h}"), ratio, named.is_some())
}

/// One hold of the hotkey: the Border shows until this drops.
pub struct Session {
    border: Border,
    bounds: (i32, i32),
    pub area: Area,
    /// The grab, the cursor at the press, and the Area at the press.
    drag: Option<(Grab, POINT, Area)>,
}

impl Session {
    pub fn begin(area: Area) -> Result<Session> {
        let r = real_monitor();
        let mut border = Border::new(r)?;
        border.show(area, None, None)?;
        Ok(Session { border, bounds: (r.right - r.left, r.bottom - r.top), area, drag: None })
    }

    /// One poll tick: a press inside the Area starts a drag, which follows the cursor until the
    /// button is up, and the Border follows the hover. Returns the Area when it changed.
    pub fn update(&mut self) -> Option<Area> {
        let a = self.area;
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        let grab_at = |a: Area, p: POINT| hit(a.w as i32, a.h as i32, p.x - a.x, p.y - a.y);
        if let Some(press) = border::take_press()
            && let Some(grab) = grab_at(a, press)
        {
            self.drag = Some((grab, press, a));
        }
        let mut guide = None;
        if let Some((grab, from, start)) = self.drag {
            if !button_held() {
                self.drag = None;
            }
            (self.area, guide) = drag(start, grab, p.x - from.x, p.y - from.y, self.bounds);
        }
        // The handle being dragged stays grown; otherwise the one under the cursor.
        let hot = self.drag.map(|(grab, ..)| grab).or_else(|| grab_at(self.area, p));
        // ponytail: a failed redraw leaves the Border stale for a tick; the Area itself is right.
        let _ = self.border.show(self.area, hot, guide);
        (self.area != a).then_some(self.area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: (i32, i32) = (5120, 1440);
    const A: Area = Area { x: 1280, y: 0, w: 2560, h: 1440 };

    #[test]
    fn hit_finds_edges_corners_and_inside() {
        assert_eq!(hit(2560, 1440, 1000, 700), Some((0, 0)));
        assert_eq!(hit(2560, 1440, 3, 700), Some((-1, 0)));
        assert_eq!(hit(2560, 1440, 2559, 0), Some((1, -1)));
        assert_eq!(hit(2560, 1440, 1000, 1435), Some((0, 1)));
        assert_eq!(hit(2560, 1440, EDGE, EDGE), Some((0, 0)));
        // Outside the Area the Border catches the mouse but nothing is grabbed.
        assert_eq!(hit(2560, 1440, -1, 700), None);
        assert_eq!(hit(2560, 1440, 2560, 700), None);
        assert_eq!(hit(2560, 1440, 1000, 1440), None);
    }

    #[test]
    fn a_move_keeps_the_size_and_stays_on_the_monitor() {
        assert_eq!(drag(A, (0, 0), 101, 5, MON), (Area { x: 1381, ..A }, None));
        assert_eq!(drag(A, (0, 0), 9000, 0, MON).0, Area { x: 2560, ..A });
        assert_eq!(drag(A, (0, 0), -9000, -9, MON).0, Area { x: 0, ..A });
    }

    #[test]
    fn a_resize_is_even_and_anchors_the_far_edge() {
        let a = Area { x: 100, y: 100, w: 1000, h: 800 };
        assert_eq!(drag(a, (1, 0), 3, 0, MON).0, Area { w: 1002, ..a });
        // Left edge: the right edge stays at 1100.
        assert_eq!(drag(a, (-1, 0), 7, 0, MON).0, Area { x: 108, w: 992, ..a });
        // Top-left corner, odd deltas both ways.
        assert_eq!(drag(a, (-1, -1), -5, 3, MON).0, Area { x: 96, y: 104, w: 1004, h: 796 });
    }

    #[test]
    fn a_resize_is_clamped_to_the_monitor_and_min() {
        let a = Area { x: 100, y: 100, w: 1000, h: 800 };
        assert_eq!(drag(a, (1, 1), 9000, 9000, MON).0, Area { w: 5020, h: 1340, ..a });
        assert_eq!(drag(a, (-1, -1), -9000, -9000, MON).0, Area { x: 0, y: 0, w: 1100, h: 900 });
        assert_eq!(drag(a, (1, 0), -9000, 0, MON).0, Area { w: MIN as u32, ..a });
        assert_eq!(drag(a, (-1, 0), 9000, 0, MON).0, Area { x: 1100 - MIN, w: MIN as u32, ..a });
    }

    #[test]
    fn a_resize_snaps_within_1_5_percent_and_guides_within_7() {
        let a = Area { x: 0, y: 0, w: 2000, h: 1440 };
        assert_eq!(drag(a, (1, 0), 540, 0, MON), (Area { w: 2560, ..a }, None)); // 2540: lands on 16:9
        assert_eq!(drag(a, (1, 0), 1420, 0, MON), (Area { w: 3440, ..a }, None)); // 3420: lands on 21:9
        // 2500 is 2.3% off 16:9: it stays free, with a guide to where it would land.
        assert_eq!(drag(a, (1, 0), 500, 0, MON), (Area { w: 2500, ..a }, Some(Area { w: 2560, ..a })));
        assert_eq!(drag(a, (1, 0), 1380, 0, MON).1, Some(Area { w: 3440, ..a })); // 3380: 1.7% off 21:9
        assert_eq!(drag(a, (1, 0), 100, 0, MON), (Area { w: 2100, ..a }, None)); // 2100: too far for a guide
        // A height drag snaps too: 1920 wide, 1090 tall → 1080.
        let b = Area { x: 0, y: 0, w: 1920, h: 1000 };
        assert_eq!(drag(b, (0, 1), 0, 90, MON).0.h, 1080);
        // A corner takes whichever side moves less: 2550×1440 → 2550×1434, not 2560×1440.
        let c = Area { x: 0, y: 0, w: 1000, h: 800 };
        assert_eq!(drag(c, (1, 1), 1550, 640, MON).0, Area { w: 2550, h: 1434, ..c });
        // No snap or guide past the monitor: 3440 from x = 1800 would end at 5240.
        let d = Area { x: 1800, y: 0, w: 3000, h: 1440 };
        assert_eq!(drag(d, (1, 0), 380, 0, MON), (Area { w: 3320, ..d }, None));
    }

    #[test]
    fn the_readout_names_a_snapped_ratio() {
        let r = |w, h, size: &str, ratio: &str, named| assert_eq!(readout(w, h), (size.into(), ratio.into(), named));
        r(2560, 1440, "2560 × 1440", "16:9", true);
        r(1778, 1000, "1778 × 1000", "16:9", true);
        r(2550, 1434, "2550 × 1434", "16:9", true);
        r(3440, 1440, "3440 × 1440", "21:9", true);
        r(3360, 1440, "3360 × 1440", "2.33", false);
        r(3294, 1440, "3294 × 1440", "2.29", false);
        r(2500, 1440, "2500 × 1440", "1.74", false);
    }
}
