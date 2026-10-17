//! The Border: while Edit mode is on, a topmost window over the whole real monitor that shades what
//! viewers won't see and draws Framelines (a cinema camera monitor's marks) around the Area. It's excluded
//! from capture, so viewers never see it. Its look is `paint_shade` and `paint_lines`; its motion is
//! `Border::show` (hover, snap) and DirectComposition animations (in and out).

use std::cell::{Cell, RefCell};
use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface, PCWSTR, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

use crate::edit::{self, Grab};
use crate::mirror::{Area, create_device};

// The look, from the approved Framelines concept.
const SHADE: D2D1_COLOR_F = rgba(5, 7, 11, 0.52);
const LINE: D2D1_COLOR_F = rgba(255, 255, 255, 0.94);
const KEYLINE: D2D1_COLOR_F = rgba(0, 0, 0, 0.42);
const MARK: D2D1_COLOR_F = rgba(255, 255, 255, 1.0);
const MARK_KEYLINE: D2D1_COLOR_F = rgba(0, 0, 0, 0.5);
const CROSS: D2D1_COLOR_F = rgba(255, 255, 255, 0.75);
const CROSS_AMBER: D2D1_COLOR_F = rgba(255, 180, 61, 0.8);
const CROSS_KEYLINE: D2D1_COLOR_F = rgba(0, 0, 0, 0.35);
const AMBER: D2D1_COLOR_F = rgba(255, 180, 61, 1.0);
const GUIDE: D2D1_COLOR_F = rgba(255, 180, 61, 0.95);
const GUIDE_KEYLINE: D2D1_COLOR_F = rgba(0, 0, 0, 0.35);
const PLATE: D2D1_COLOR_F = rgba(9, 11, 15, 0.8);
const RATIO: D2D1_COLOR_F = rgba(181, 186, 196, 1.0);
/// The readout's text size and the gaps around it, in px.
const TEXT_PX: f32 = 17.0;
/// A handle under the cursor grows by this much from its corner or centre.
const GROW: f32 = 0.18;

// The motion, in ms, with CSS cubic-bezier curves.
const GROW_MS: f32 = 140.0;
const GROW_EASE: [f32; 4] = [0.2, 0.8, 0.2, 1.0];
const AMBER_MS: f32 = 120.0;
const AMBER_EASE: [f32; 4] = [0.25, 0.1, 0.25, 1.0]; // CSS `ease`
const SHADE_IN_MS: f32 = 180.0;
const LINES_IN_MS: f32 = 200.0;
/// The lines settle onto the Area from this much larger.
const SETTLE_FROM: f32 = 1.015;
const IN_EASE: [f32; 4] = [0.16, 1.0, 0.3, 1.0];
const OUT_MS: f32 = 110.0;
const OUT_EASE: [f32; 4] = [0.42, 0.0, 1.0, 1.0]; // CSS `ease-in`
/// The named ratio blinks once when a resize lands on it.
const BLINK_MS: f32 = 220.0;

/// The handles: corners, then edge midpoints. A Grab names each.
pub const HANDLES: [Grab; 8] = [(-1, -1), (1, -1), (-1, 1), (1, 1), (0, -1), (1, 0), (0, 1), (-1, 0)];

const FADE_TIMER: usize = 1;

thread_local! {
    /// The last press on the Border, in desktop pixels, until Edit mode takes it.
    static PRESS: Cell<Option<POINT>> = const { Cell::new(None) };
    /// The Area on show, for the cursor shape.
    static SHOWN: Cell<Option<Area>> = const { Cell::new(None) };
    /// A Border fading out after Edit mode ended, until its timer destroys it.
    static FADING: RefCell<Option<Window>> = const { RefCell::new(None) };
}

pub fn take_press() -> Option<POINT> {
    PRESS.take()
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
}

fn mix(a: D2D1_COLOR_F, b: D2D1_COLOR_F, t: f32) -> D2D1_COLOR_F {
    let m = |x: f32, y: f32| x + (y - x) * t;
    D2D1_COLOR_F { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: m(a.a, b.a) }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at time `t` (0 to 1).
fn bezier([x1, y1, x2, y2]: [f32; 4], t: f32) -> f32 {
    let at = |p1: f32, p2: f32, u: f32| 3.0 * (1.0 - u) * (1.0 - u) * u * p1 + 3.0 * (1.0 - u) * u * u * p2 + u * u * u;
    if t <= 0.0 || t >= 1.0 {
        return t.clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..24 {
        let u = (lo + hi) / 2.0;
        if at(x1, x2, u) < t { lo = u } else { hi = u }
    }
    at(y1, y2, (lo + hi) / 2.0)
}

/// Everything one frame of the Border shows, in monitor pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub screen: (i32, i32),
    pub area: Area,
    /// Where a resize near a snap ratio would land.
    pub guide: Option<Area>,
    /// How grown each of HANDLES is, 0 to 1.
    pub grow: [f32; 8],
    /// The outline and marks from white (0) to amber (1).
    pub amber: f32,
    /// The ratio's opacity.
    pub ratio: f32,
}

/// The device-independent pieces painting needs.
pub struct Kit {
    factory: ID2D1Factory1,
    square: ID2D1StrokeStyle,
    dashed: ID2D1StrokeStyle,
    dwrite: IDWriteFactory,
    /// The readout's size, ratio and named-ratio faces.
    size: IDWriteTextFormat,
    ratio: IDWriteTextFormat,
    named: IDWriteTextFormat,
    tabular: IDWriteTypography,
}

impl Kit {
    pub fn new() -> Result<Kit> {
        unsafe {
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let style = |cap, dash| D2D1_STROKE_STYLE_PROPERTIES {
                startCap: cap,
                endCap: cap,
                dashCap: D2D1_CAP_STYLE_FLAT,
                lineJoin: D2D1_LINE_JOIN_MITER,
                miterLimit: 10.0,
                dashStyle: dash,
                dashOffset: 0.0,
            };
            let square = ID2D1Factory::CreateStrokeStyle(&factory, &style(D2D1_CAP_STYLE_SQUARE, D2D1_DASH_STYLE_SOLID), None)?;
            // Chrome's dashed 2 px border: 6 px dashes, 6 px gaps.
            let dashed = ID2D1Factory::CreateStrokeStyle(&factory, &style(D2D1_CAP_STYLE_FLAT, D2D1_DASH_STYLE_CUSTOM), Some(&[3.0, 3.0]))?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let format = |weight| -> Result<IDWriteTextFormat> {
                let f = dwrite.CreateTextFormat(
                    w!("Bahnschrift"),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_SEMI_CONDENSED,
                    TEXT_PX,
                    w!("en-us"),
                )?;
                // A TEXT_PX line (CSS `line-height: 1`) with the numerals centred in it, as in the concept.
                f.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, TEXT_PX, (TEXT_PX + cap_height(&dwrite, weight)?) / 2.0)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                Ok(f)
            };
            let tabular = dwrite.CreateTypography()?;
            tabular.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, parameter: 1 })?;
            Ok(Kit {
                size: format(DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
                ratio: format(DWRITE_FONT_WEIGHT_MEDIUM)?,
                named: format(DWRITE_FONT_WEIGHT_BOLD)?,
                factory,
                square,
                dashed,
                dwrite,
                tabular,
            })
        }
    }

    unsafe fn text(&self, s: &str, format: &IDWriteTextFormat) -> Result<(IDWriteTextLayout, f32)> {
        unsafe {
            let s: Vec<u16> = s.encode_utf16().collect();
            let layout = self.dwrite.CreateTextLayout(&s, format, 1000.0, TEXT_PX)?;
            layout.SetTypography(&self.tabular, DWRITE_TEXT_RANGE { startPosition: 0, length: s.len() as u32 })?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            Ok((layout, m.width))
        }
    }

    /// An open polyline per figure.
    unsafe fn path(&self, figures: &[&[(f32, f32)]]) -> Result<ID2D1PathGeometry> {
        unsafe {
            let g = ID2D1Factory::CreatePathGeometry(&self.factory)?;
            let sink = g.Open()?;
            for f in figures {
                sink.BeginFigure(Vector2::new(f[0].0, f[0].1), D2D1_FIGURE_BEGIN_HOLLOW);
                for &(x, y) in &f[1..] {
                    sink.AddLine(Vector2::new(x, y));
                }
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            sink.Close()?;
            Ok(g)
        }
    }
}

/// Bahnschrift SemiCondensed's cap height at TEXT_PX.
unsafe fn cap_height(dwrite: &IDWriteFactory, weight: DWRITE_FONT_WEIGHT) -> Result<f32> {
    unsafe {
        let mut fonts = None;
        dwrite.GetSystemFontCollection(&mut fonts, false)?;
        let fonts = fonts.unwrap();
        let (mut i, mut found) = (0, BOOL(0));
        fonts.FindFamilyName(w!("Bahnschrift"), &mut i, &mut found)?;
        if !found.as_bool() {
            return Ok(TEXT_PX * 0.7);
        }
        let font = fonts.GetFontFamily(i)?.GetFirstMatchingFont(weight, DWRITE_FONT_STRETCH_SEMI_CONDENSED, DWRITE_FONT_STYLE_NORMAL)?;
        let mut m = DWRITE_FONT_METRICS::default();
        font.GetMetrics(&mut m);
        Ok(m.capHeight as f32 * TEXT_PX / m.designUnitsPerEm as f32)
    }
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: l, top: t, right: r, bottom: b }
}

/// The Area's left, top, right and bottom.
fn edges(a: Area) -> (f32, f32, f32, f32) {
    (a.x as f32, a.y as f32, (a.x + a.w as i32) as f32, (a.y + a.h as i32) as f32)
}

/// What viewers won't see: everything on the screen outside the Area, shaded to about half.
pub unsafe fn paint_shade(rt: &ID2D1RenderTarget, look: &Look) -> Result<()> {
    unsafe {
        let (sw, sh) = (look.screen.0 as f32, look.screen.1 as f32);
        let (l, t, r, b) = edges(look.area);
        let brush = rt.CreateSolidColorBrush(&SHADE, None)?;
        for band in [rect(0.0, 0.0, sw, t), rect(0.0, b, sw, sh), rect(0.0, t, l, b), rect(r, t, sw, b)] {
            rt.FillRectangle(&band, &brush);
        }
        Ok(())
    }
}

/// The Framelines around the Area: the outline, the snap guide, the readout, the handles and the
/// centre cross, in that order from the bottom.
pub unsafe fn paint_lines(rt: &ID2D1RenderTarget, kit: &Kit, look: &Look) -> Result<()> {
    unsafe {
        let a = look.area;
        let (l, t, r, b) = edges(a);
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        let brush = rt.CreateSolidColorBrush(&LINE, None)?;
        let ink = |c: D2D1_COLOR_F| {
            brush.SetColor(&c);
            &brush
        };
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

        // The outline: a 2 px line just inside the Area with 1 px keylines on both sides, so it reads by
        // brightness on any picture. Each is one stroke, so the corners aren't drawn twice.
        rt.DrawRectangle(&rect(l - 0.5, t - 0.5, r + 0.5, b + 0.5), ink(KEYLINE), 1.0, None);
        rt.DrawRectangle(&rect(l + 1.0, t + 1.0, r - 1.0, b - 1.0), ink(mix(LINE, AMBER, look.amber)), 2.0, None);
        rt.DrawRectangle(&rect(l + 2.5, t + 2.5, r - 2.5, b - 2.5), ink(KEYLINE), 1.0, None);

        if let Some(g) = look.guide {
            let (gl, gt, gr, gb) = edges(g);
            rt.DrawRectangle(&rect(gl - 0.5, gt - 0.5, gr + 0.5, gb + 0.5), ink(GUIDE_KEYLINE), 1.0, None);
            rt.DrawRectangle(&rect(gl + 1.0, gt + 1.0, gr - 1.0, gb - 1.0), ink(GUIDE), 2.0, &kit.dashed);
        }

        // The readout, top-left past the corner bracket: W × H, then the ratio.
        let (size, ratio, named) = edit::readout(a.w, a.h);
        let (size, size_w) = kit.text(&size, &kit.size)?;
        let (ratio, ratio_w) = kit.text(&ratio, if named { &kit.named } else { &kit.ratio })?;
        let (px, py) = (l + 58.0, t + 16.0);
        let plate = D2D1_ROUNDED_RECT { rect: rect(px, py, px + 10.0 + size_w + 11.0 + ratio_w + 10.0, py + 6.0 + TEXT_PX + 6.0), radiusX: 4.0, radiusY: 4.0 };
        rt.FillRoundedRectangle(&plate, ink(PLATE));
        rt.DrawTextLayout(Vector2::new(px + 10.0, py + 6.0), &size, ink(MARK), D2D1_DRAW_TEXT_OPTIONS_NONE);
        let mut c = if named { AMBER } else { RATIO };
        c.a *= look.ratio;
        rt.DrawTextLayout(Vector2::new(px + 10.0 + size_w + 11.0, py + 6.0), &ratio, ink(c), D2D1_DRAW_TEXT_OPTIONS_NONE);

        // The handles: corner brackets and edge ticks, each a white stroke on a dark one, grown from
        // its corner (a bracket) or its centre (a tick).
        let mut base = Matrix3x2::default();
        rt.GetTransform(&mut base);
        let mark = mix(MARK, AMBER, look.amber);
        for (&(gx, gy), grow) in HANDLES.iter().zip(look.grow) {
            let pick = |g: i32, lo: f32, mid: f32, hi: f32| [lo, mid, hi][(g + 1) as usize];
            let (x, y) = (pick(gx, l, cx, r), pick(gy, t, cy, b));
            // Inward from the handle's edge.
            let (ix, iy) = (-gx as f32, -gy as f32);
            let (points, origin) = match (gx, gy) {
                (0, _) => (vec![(x, y + iy * 3.0), (x, y + iy * 21.0)], (x, y + iy * 12.0)),
                (_, 0) => (vec![(x + ix * 3.0, y), (x + ix * 21.0, y)], (x + ix * 12.0, y)),
                _ => (vec![(x + ix * 3.0, y + iy * 44.0), (x + ix * 3.0, y + iy * 3.0), (x + ix * 44.0, y + iy * 3.0)], (x, y)),
            };
            let s = 1.0 + GROW * grow;
            rt.SetTransform(&(Matrix3x2::scale_around(s, s, Vector2::new(origin.0, origin.1)) * base));
            let path = kit.path(&[&points])?;
            rt.DrawGeometry(&path, ink(MARK_KEYLINE), 7.0, &kit.square);
            rt.DrawGeometry(&path, ink(mark), 4.0, &kit.square);
        }
        rt.SetTransform(&base);

        let cross = kit.path(&[&[(cx, cy - 12.0), (cx, cy + 12.0)], &[(cx - 12.0, cy), (cx + 12.0, cy)]])?;
        rt.DrawGeometry(&cross, ink(CROSS_KEYLINE), 3.5, &kit.square);
        rt.DrawGeometry(&cross, ink(mix(CROSS, CROSS_AMBER, look.amber)), 1.5, &kit.square);
        Ok(())
    }
}

/// A value easing from one target to the next, like a CSS transition.
#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    at: Instant,
    ms: f32,
    ease: [f32; 4],
}

impl Tween {
    fn new(v: f32, ms: f32, ease: [f32; 4]) -> Tween {
        Tween { from: v, to: v, at: Instant::now(), ms, ease }
    }

    fn value(&self, now: Instant) -> f32 {
        let t = (now - self.at).as_secs_f32() * 1000.0 / self.ms;
        if t >= 1.0 { self.to } else { self.from + (self.to - self.from) * bezier(self.ease, t) }
    }

    fn aim(&mut self, to: f32, now: Instant) {
        if to != self.to {
            *self = Tween { from: self.value(now), to, at: now, ..*self };
        }
    }
}

/// The window and its composition: a shade layer under a lines layer.
struct Window {
    hwnd: HWND,
    dcomp: IDCompositionDesktopDevice,
    _target: IDCompositionTarget,
    /// Fades the whole Border out.
    fade: IDCompositionEffectGroup,
    shade: (IDCompositionSurface, IDCompositionEffectGroup),
    lines: (IDCompositionSurface, IDCompositionEffectGroup),
    settle: IDCompositionScaleTransform,
    kit: Kit,
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

pub struct Border {
    win: Option<Window>,
    screen: (i32, i32),
    drawn: Option<Look>,
    grow: [Tween; 8],
    amber: Tween,
    blink: Option<Instant>,
}

impl Border {
    /// A hidden Border over the `monitor` rect; the first `show` brings it in.
    pub fn new(monitor: RECT) -> Result<Border> {
        // One that's still fading out goes at once.
        drop(FADING.take());
        let (w, h) = (monitor.right - monitor.left, monitor.bottom - monitor.top);
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            let class = w!("cropcast-border");
            // Fails harmlessly as "already registered" from the second Edit mode on.
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance, lpszClassName: class, ..Default::default() });
            // No redirection bitmap: DirectComposition draws it, so a monitor-sized shade costs no
            // monitor-sized copy per frame.
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("cropcast Border"),
                WS_POPUP,
                monitor.left,
                monitor.top,
                w,
                h,
                None,
                None,
                Some(instance),
                None,
            )?;
            // Owned from here, so a failure below destroys the window too.
            let guard = HwndGuard(hwnd);
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)?;

            let kit = Kit::new()?;
            let dxgi: IDXGIDevice = create_device()?.device.cast()?;
            let d2d = kit.factory.CreateDevice(&dxgi)?;
            let dcomp: IDCompositionDesktopDevice = DCompositionCreateDevice2(&d2d)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let root = dcomp.CreateVisual()?;
            let fade = dcomp.CreateEffectGroup()?;
            root.SetEffect(&fade)?;
            let layer = || -> Result<(IDCompositionSurface, IDCompositionEffectGroup, IDCompositionVisual2)> {
                let surface = dcomp.CreateSurface(w as u32, h as u32, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED)?;
                let visual = dcomp.CreateVisual()?;
                let effect = dcomp.CreateEffectGroup()?;
                visual.SetContent(&surface)?;
                visual.SetEffect(&effect)?;
                root.AddVisual(&visual, true, None)?;
                Ok((surface, effect, visual))
            };
            let (shade, shade_fx, _) = layer()?;
            let (lines, lines_fx, lines_visual) = layer()?;
            let settle = dcomp.CreateScaleTransform()?;
            lines_visual.SetTransform(&settle)?;
            lines_visual.SetBitmapInterpolationMode(DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR)?;
            target.SetRoot(&root)?;
            std::mem::forget(guard);
            PRESS.set(None);
            Ok(Border {
                win: Some(Window { hwnd, dcomp, _target: target, fade, shade: (shade, shade_fx), lines: (lines, lines_fx), settle, kit }),
                screen: (w, h),
                drawn: None,
                grow: [Tween::new(0.0, GROW_MS, GROW_EASE); 8],
                amber: Tween::new(0.0, AMBER_MS, AMBER_EASE),
                blink: None,
            })
        }
    }

    /// Lets the mouse through, for a Border that's only shown, not edited.
    pub fn pass_clicks(&self) {
        if let Some(win) = &self.win {
            unsafe {
                let ex = GetWindowLongPtrW(win.hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(win.hwnd, GWL_EXSTYLE, ex | WS_EX_TRANSPARENT.0 as isize);
            }
        }
    }

    /// Shows the Border around `area`, with `hot` grown and the snap `guide`. Called every Edit mode
    /// tick: it redraws only while something changes or moves.
    pub fn show(&mut self, area: Area, hot: Option<Grab>, guide: Option<Area>) -> Result<()> {
        let now = Instant::now();
        let named = edit::readout(area.w, area.h).2;
        if named && self.drawn.is_some_and(|d| !edit::readout(d.area.w, d.area.h).2) {
            self.blink = Some(now);
        }
        for (g, tween) in HANDLES.iter().zip(&mut self.grow) {
            tween.aim(if hot == Some(*g) { 1.0 } else { 0.0 }, now);
        }
        self.amber.aim(if named { 1.0 } else { 0.0 }, now);
        // CSS steps(2) over a 1 → 0.25 → 1 blink.
        let ratio = match self.blink.map(|at| (now - at).as_secs_f32() * 1000.0 / BLINK_MS) {
            Some(t) if t < 1.0 => [1.0, 0.625, 0.25, 0.625][(t * 4.0) as usize],
            _ => 1.0,
        };
        let look = Look {
            screen: self.screen,
            area,
            guide,
            grow: self.grow.map(|g| g.value(now)),
            amber: self.amber.value(now),
            ratio,
        };
        if self.drawn == Some(look) {
            return Ok(());
        }
        let Some(win) = &self.win else { return Ok(()) };
        unsafe {
            if self.drawn.is_none_or(|d| d.area != area) {
                draw(&win.shade.0, self.screen, |rt| paint_shade(rt, &look))?;
            }
            draw(&win.lines.0, self.screen, |rt| paint_lines(rt, &win.kit, &look))?;
            if self.drawn.is_none() {
                // In: the shade fades up and the lines settle onto the Area from slightly larger.
                let (cx, cy) = (area.x as f32 + area.w as f32 / 2.0, area.y as f32 + area.h as f32 / 2.0);
                win.shade.1.SetOpacity(&eased(&win.dcomp, 0.0, 1.0, SHADE_IN_MS, IN_EASE)?)?;
                win.lines.1.SetOpacity(&eased(&win.dcomp, 0.0, 1.0, LINES_IN_MS, IN_EASE)?)?;
                win.settle.SetCenterX2(cx)?;
                win.settle.SetCenterY2(cy)?;
                let settle = eased(&win.dcomp, SETTLE_FROM, 1.0, LINES_IN_MS, IN_EASE)?;
                win.settle.SetScaleX(&settle)?;
                win.settle.SetScaleY(&settle)?;
            }
            win.dcomp.Commit()?;
            if self.drawn.is_none() {
                let _ = ShowWindow(win.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        self.drawn = Some(look);
        SHOWN.set(Some(area));
        Ok(())
    }
}

impl Drop for Border {
    /// Fades the Border out; its timer destroys the window and lets go of the GPU after.
    fn drop(&mut self) {
        SHOWN.set(None);
        let Some(win) = self.win.take() else { return };
        if self.drawn.is_none() {
            return;
        }
        unsafe {
            let out = eased(&win.dcomp, 1.0, 0.0, OUT_MS, OUT_EASE).and_then(|a| win.fade.SetOpacity(&a)).and_then(|_| win.dcomp.Commit());
            if out.is_ok() && SetTimer(Some(win.hwnd), FADE_TIMER, OUT_MS as u32 + 10, None) != 0 {
                FADING.set(Some(win));
            }
        }
    }
}

struct HwndGuard(HWND);

impl Drop for HwndGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

/// Redraws all of a `size` `surface` with `paint`.
unsafe fn draw(surface: &IDCompositionSurface, size: (i32, i32), paint: impl FnOnce(&ID2D1RenderTarget) -> Result<()>) -> Result<()> {
    unsafe {
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = surface.BeginDraw(None, &mut offset)?;
        // DirectComposition may hand out a piece of a bigger atlas: draw and clear only ours.
        dc.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
        dc.PushAxisAlignedClip(&rect(0.0, 0.0, size.0 as f32, size.1 as f32), D2D1_ANTIALIAS_MODE_ALIASED);
        dc.Clear(None);
        let painted = paint(&dc);
        dc.PopAxisAlignedClip();
        surface.EndDraw()?;
        painted
    }
}

/// A DirectComposition animation from `from` to `to` over `ms` along a CSS cubic-bezier, as short
/// linear pieces: the compositor plays it with no frames from here.
unsafe fn eased(dcomp: &IDCompositionDesktopDevice, from: f32, to: f32, ms: f32, ease: [f32; 4]) -> Result<IDCompositionAnimation> {
    const PIECES: usize = 24;
    unsafe {
        let a = dcomp.CreateAnimation()?;
        let secs = ms as f64 / 1000.0;
        let at = |i: usize| from + (to - from) * bezier(ease, i as f32 / PIECES as f32);
        for i in 0..PIECES {
            let (v0, v1) = (at(i), at(i + 1));
            a.AddCubic(secs * i as f64 / PIECES as f64, v0, (v1 - v0) / (secs as f32 / PIECES as f32), 0.0, 0.0)?;
        }
        a.End(secs, to)?;
        Ok(a)
    }
}

fn cursor_for(g: Grab) -> PCWSTR {
    match g {
        (0, 0) => IDC_SIZEALL,
        (_, 0) => IDC_SIZEWE,
        (0, _) => IDC_SIZENS,
        (x, y) if x == y => IDC_SIZENWSE,
        _ => IDC_SIZENESW,
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR => {
                let (mut p, mut r) = (POINT::default(), RECT::default());
                let _ = GetCursorPos(&mut p);
                let _ = GetWindowRect(hwnd, &mut r);
                let (x, y) = (p.x - r.left, p.y - r.top);
                let g = SHOWN.get().and_then(|a| edit::hit(a.w as i32, a.h as i32, x - a.x, y - a.y));
                SetCursor(LoadCursorW(None, g.map_or(IDC_ARROW, cursor_for)).ok());
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                // The rest of the drag is polled: a window that never takes focus can't hold mouse capture.
                let mut r = RECT::default();
                let _ = GetWindowRect(hwnd, &mut r);
                let (x, y) = (lp.0 as i16 as i32, (lp.0 >> 16) as i16 as i32);
                PRESS.set(Some(POINT { x: r.left + x, y: r.top + y }));
                LRESULT(0)
            }
            WM_TIMER if wp.0 == FADE_TIMER => {
                // Faded out: the window and everything on the GPU go.
                drop(FADING.take());
                LRESULT(0)
            }
            WM_DPICHANGED => LRESULT(0), // keep the exact pixel size on any monitor
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::*;
    use windows::core::HSTRING;

    #[test]
    fn bezier_matches_css_ends_and_middle() {
        assert_eq!(bezier(IN_EASE, 0.0), 0.0);
        assert!((bezier(IN_EASE, 1.0) - 1.0).abs() < 1e-4);
        // `ease-in` is slow to start, the settle fast.
        assert!(bezier(OUT_EASE, 0.25) < 0.15);
        assert!(bezier(IN_EASE, 0.25) > 0.75);
    }

    /// Renders the Border's states over the concept's scenes, offscreen, to PNGs for the eye:
    /// `CROPCAST_SHOTS=<dir with scenes/*.png> cargo test --release -- --ignored render`.
    #[test]
    #[ignore]
    fn render_looks() {
        let dir = std::path::PathBuf::from(std::env::var("CROPCAST_SHOTS").expect("CROPCAST_SHOTS"));
        std::fs::create_dir_all(dir.join("native")).unwrap();
        let a = |x, y, w, h| Area { x, y, w, h };
        let hover = |i: usize| {
            let mut g = [0.0; 8];
            g[i] = 1.0;
            g
        };
        let looks = [
            ("default-snow", "snow", a(1280, 0, 2560, 1440), None, [0.0; 8]),
            ("hover-east-night", "night", a(1280, 0, 2400, 1440), None, hover(5)),
            ("hover-nw-night", "night", a(1280, 0, 2400, 1440), None, hover(0)),
            ("near-forest", "forest", a(1280, 0, 3294, 1440), Some(a(1280, 0, 3440, 1440)), [0.0; 8]),
            ("landed-21-9-forest", "forest", a(1280, 0, 3440, 1440), None, [0.0; 8]),
            ("landed-16-9-snow", "snow", a(600, 200, 1920, 1080), None, [0.0; 8]),
        ];
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).unwrap();
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let kit = Kit::new().unwrap();
            for (name, scene, area, guide, grow) in looks {
                let path = HSTRING::from(dir.join("scenes").join(format!("{scene}.png")).as_os_str());
                let frame = wic.CreateDecoderFromFilename(&path, None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).unwrap().GetFrame(0).unwrap();
                let conv = wic.CreateFormatConverter().unwrap();
                conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).unwrap();
                let bmp = wic.CreateBitmapFromSourceRect(&conv, 0, 0, 5120, 1440).unwrap();
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    ..Default::default()
                };
                let rt = kit.factory.CreateWicBitmapRenderTarget(&bmp, &props).unwrap();
                let named = edit::readout(area.w, area.h).2;
                let look = Look { screen: (5120, 1440), area, guide, grow, amber: if named { 1.0 } else { 0.0 }, ratio: 1.0 };
                rt.BeginDraw();
                paint_shade(&rt, &look).unwrap();
                paint_lines(&rt, &kit, &look).unwrap();
                rt.EndDraw(None, None).unwrap();
                save(&wic, &bmp, &dir.join("native").join(format!("{name}.png")));
            }
        }
    }

    pub unsafe fn save(wic: &IWICImagingFactory, bmp: &IWICBitmap, path: &std::path::Path) {
        unsafe {
            let stream = wic.CreateStream().unwrap();
            stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0).unwrap();
            let enc = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).unwrap();
            enc.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
            let mut frame = None;
            enc.CreateNewFrame(&mut frame, std::ptr::null_mut()).unwrap();
            let frame = frame.unwrap();
            frame.Initialize(None).unwrap();
            frame.WriteSource(bmp, std::ptr::null()).unwrap();
            frame.Commit().unwrap();
            enc.Commit().unwrap();
        }
    }
}
