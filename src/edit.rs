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
const MIN: i32 = 360;
/// A resize lands on a snap ratio when the dragged side is this close to it, in px. A dial to tune.
const SNAP_PX: i32 = 24;
/// (21, 9) is true 21:9 (3360×1440); (43, 18) would land on the 3440×1440 ultrawide instead.
const SNAP_RATIOS: [(i32, i32); 2] = [(16, 9), (21, 9)];

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

/// The grab at (`x`, `y`) inside a `w`×`h` Area, relative to its top-left.
pub fn hit(w: i32, h: i32, x: i32, y: i32) -> Grab {
    let side = |p: i32, len: i32| if p < EDGE { -1 } else if p >= len - EDGE { 1 } else { 0 };
    (side(x, w), side(y, h))
}

/// `num / den` rounded to the nearest even number.
fn even(num: i32, den: i32) -> i32 {
    (num + den) / (2 * den) * 2
}

/// `len` pulled onto a snap ratio against `other` when it's within SNAP_PX and still fits `max`.
fn snap(len: i32, other: i32, wide: bool, max: i32) -> i32 {
    SNAP_RATIOS
        .iter()
        .map(|&(n, d)| if wide { even(other * n, d) } else { even(other * d, n) })
        .find(|&t| (t - len).abs() <= SNAP_PX && (MIN..=max).contains(&t))
        .unwrap_or(len)
}

/// The Area after dragging `grab` of `from` by (`dx`, `dy`) on a `bounds` monitor: moved or resized,
/// kept on the monitor, at least MIN, snapped, and even.
pub fn drag(from: Area, grab: Grab, dx: i32, dy: i32, bounds: (i32, i32)) -> Area {
    let (mw, mh) = bounds;
    let (gx, gy) = grab;
    if grab == (0, 0) {
        let x = (from.x + dx).min(mw - from.w as i32).max(0);
        let y = (from.y + dy).min(mh - from.h as i32).max(0);
        return Area { x, y, ..from };
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
    let (mut w, mut h) = (r - l, b - t);
    // ponytail: a corner snaps its width only; the height stays where the cursor put it.
    if gx != 0 {
        w = snap(w, h, true, max_w);
    } else {
        h = snap(h, w, false, max_h);
    }
    let (w, h) = (w & !1, h & !1);
    Area {
        x: if gx < 0 { r - w } else { l },
        y: if gy < 0 { b - h } else { t },
        w: w as u32,
        h: h as u32,
    }
}

/// The size label: "2560 × 1440", plus the ratio when the Area sits on a snap.
pub fn label(w: u32, h: u32) -> String {
    let (w, h) = (w as i32, h as i32);
    match SNAP_RATIOS.iter().find(|&&(n, d)| w == even(h * n, d)) {
        Some((n, d)) => format!("{w} × {h}  {n}:{d}"),
        None => format!("{w} × {h}"),
    }
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
        let mut border = Border::new()?;
        border.show(area)?;
        Ok(Session { border, bounds: (r.right - r.left, r.bottom - r.top), area, drag: None })
    }

    /// One poll tick: a press on the Border starts a drag, which follows the cursor until the button
    /// is up. Returns the Area when it changed.
    pub fn update(&mut self) -> Option<Area> {
        let a = self.area;
        if let Some(p) = border::take_press() {
            self.drag = Some((hit(a.w as i32, a.h as i32, p.x - a.x, p.y - a.y), p, a));
        }
        let (grab, from, start) = self.drag?;
        let mut p = from;
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        if !button_held() {
            self.drag = None;
        }
        self.area = drag(start, grab, p.x - from.x, p.y - from.y, self.bounds);
        if self.area == a {
            return None;
        }
        // ponytail: a failed redraw leaves the Border stale for a tick; the Area itself is right.
        let _ = self.border.show(self.area);
        Some(self.area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: (i32, i32) = (5120, 1440);
    const A: Area = Area { x: 1280, y: 0, w: 2560, h: 1440 };

    #[test]
    fn hit_finds_edges_corners_and_inside() {
        assert_eq!(hit(2560, 1440, 1000, 700), (0, 0));
        assert_eq!(hit(2560, 1440, 3, 700), (-1, 0));
        assert_eq!(hit(2560, 1440, 2559, 0), (1, -1));
        assert_eq!(hit(2560, 1440, 1000, 1435), (0, 1));
        assert_eq!(hit(2560, 1440, EDGE, EDGE), (0, 0));
    }

    #[test]
    fn a_move_keeps_the_size_and_stays_on_the_monitor() {
        assert_eq!(drag(A, (0, 0), 101, 5, MON), Area { x: 1381, ..A });
        assert_eq!(drag(A, (0, 0), 9000, 0, MON), Area { x: 2560, ..A });
        assert_eq!(drag(A, (0, 0), -9000, -9, MON), Area { x: 0, ..A });
    }

    #[test]
    fn a_resize_is_even_and_anchors_the_far_edge() {
        let a = Area { x: 100, y: 100, w: 1000, h: 800 };
        assert_eq!(drag(a, (1, 0), 3, 0, MON), Area { w: 1002, ..a });
        // Left edge: the right edge stays at 1100.
        assert_eq!(drag(a, (-1, 0), 7, 0, MON), Area { x: 108, w: 992, ..a });
        // Top-left corner, odd deltas both ways.
        assert_eq!(drag(a, (-1, -1), -5, 3, MON), Area { x: 96, y: 104, w: 1004, h: 796 });
    }

    #[test]
    fn a_resize_is_clamped_to_the_monitor_and_min() {
        let a = Area { x: 100, y: 100, w: 1000, h: 800 };
        assert_eq!(drag(a, (1, 1), 9000, 9000, MON), Area { w: 5020, h: 1340, ..a });
        assert_eq!(drag(a, (-1, -1), -9000, -9000, MON), Area { x: 0, y: 0, w: 1100, h: 900 });
        assert_eq!(drag(a, (1, 0), -9000, 0, MON), Area { w: MIN as u32, ..a });
        assert_eq!(drag(a, (-1, 0), 9000, 0, MON), Area { x: 1100 - MIN, w: MIN as u32, ..a });
    }

    #[test]
    fn a_resize_snaps_near_16_9_and_21_9() {
        let a = Area { x: 0, y: 0, w: 2000, h: 1440 };
        assert_eq!(drag(a, (1, 0), 540, 0, MON).w, 2560); // 2540 → 16:9
        assert_eq!(drag(a, (1, 0), 1380, 0, MON).w, 3360); // 3380 → 21:9
        assert_eq!(drag(a, (1, 0), 500, 0, MON).w, 2500); // too far: free
        // A height drag snaps too: 1920 wide, 1090 tall → 1080.
        let b = Area { x: 0, y: 0, w: 1920, h: 1000 };
        assert_eq!(drag(b, (0, 1), 0, 90, MON).h, 1080);
        // No snap past the monitor: 3360 from x = 1800 would end at 5160.
        let c = Area { x: 1800, y: 0, w: 3000, h: 1440 };
        assert_eq!(drag(c, (1, 0), 380, 0, MON).w, 3320);
    }

    #[test]
    fn the_label_names_a_snapped_ratio() {
        assert_eq!(label(2560, 1440), "2560 × 1440  16:9");
        assert_eq!(label(1778, 1000), "1778 × 1000  16:9");
        assert_eq!(label(3360, 1440), "3360 × 1440  21:9");
        assert_eq!(label(2500, 1440), "2500 × 1440");
    }
}
