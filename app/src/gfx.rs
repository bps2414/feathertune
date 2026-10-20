//! Direct2D/DirectWrite drawing shared by the native title bar (titlebar.rs) and the mini player
//! (miniplayer.rs), plus their colors: system light/dark and the album theme (`Theme`, set by
//! album_color.rs, with a short fade between two themes).
//!
//! Each window paints in WM_PAINT through a DC render target bound to the paint HDC, so the drawing
//! is clipped like GDI (the title bar never draws over the child webview below it). Everything is in
//! DIPs: the target gets the window's DPI. Icons are glyphs of "Segoe MDL2 Assets" (Windows 10 and 11).
//! Both windows live on the main thread, so the factories are per thread.

use std::cell::OnceCell;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_GAMMA_1_0, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1CreateFactory, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1LinearGradientBrush,
    ID2D1RadialGradientBrush, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory,
    IDWriteTextFormat, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::core::{HSTRING, Result, w};
use windows_numerics::Vector2;

pub const ICON_FONT: &str = "Segoe MDL2 Assets";
pub const TEXT_FONT: &str = "Segoe UI";

#[derive(Clone)]
struct Factories {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
}

thread_local! {
    static FACTORIES: OnceCell<Factories> = const { OnceCell::new() };
}

fn factories() -> Result<Factories> {
    FACTORIES.with(|cell| {
        if let Some(f) = cell.get() {
            return Ok(f.clone());
        }
        let f = unsafe {
            Factories {
                d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                dwrite: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            }
        };
        Ok(cell.get_or_init(|| f).clone())
    })
}

/// A single-line text format, vertically centered, cut with an ellipsis.
pub fn text_format(
    family: &str,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    align: DWRITE_TEXT_ALIGNMENT,
) -> Result<IDWriteTextFormat> {
    let dwrite = factories()?.dwrite;
    unsafe {
        let format = dwrite.CreateTextFormat(
            &HSTRING::from(family),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        format.SetTextAlignment(align)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let ellipsis = dwrite.CreateEllipsisTrimmingSign(&format)?;
        let trimming =
            DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        format.SetTrimming(&trimming, &ellipsis)?;
        Ok(format)
    }
}

/// A text layout of `text` in a box of `width` × `height` DIPs, for styling ranges of it.
pub fn text_layout(text: &str, format: &IDWriteTextFormat, width: f32, height: f32) -> Result<IDWriteTextLayout> {
    let text: Vec<u16> = text.encode_utf16().collect();
    unsafe { factories()?.dwrite.CreateTextLayout(&text, format, width, height) }
}

pub const fn point(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub const fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

pub const fn rgb(hex: u32) -> D2D1_COLOR_F {
    rgba(hex, 1.0)
}

pub const fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}

pub fn with_alpha(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: c.a * a, ..c }
}

/// The window's scale factor (1.0 at 96 DPI).
pub fn scale(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// Colors of our own surfaces.
#[derive(Clone, Copy)]
pub struct Palette {
    pub background: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    /// Button hover and other soft fills.
    pub hover: D2D1_COLOR_F,
    /// The "app mode" of Windows, whatever the theme.
    pub system_dark: bool,
}

/// The colors of the "ink in water" look, taken from the artwork: a deep ink and the four glows (the
/// first is the dominant color). The ink is always dark, so the white text on it is readable.
#[derive(Clone, Copy, PartialEq)]
pub struct Theme {
    pub ink: u32,
    pub glows: [u32; 4],
}

/// How long the colors take to flow from one theme to the next.
const FADE: Duration = Duration::from_millis(700);

struct Fade {
    from: Option<Theme>,
    to: Option<Theme>,
    /// `None`: settled on `to`.
    start: Option<Instant>,
}

static THEME: Mutex<Fade> = Mutex::new(Fade { from: None, to: None, start: None });

/// The theme being shown and the one it flows from (`None`: the default look), with the progress `t`
/// (0..1, eased) of the flow.
pub struct Blend {
    pub from: Option<Theme>,
    pub to: Option<Theme>,
    pub t: f32,
}

pub fn blend() -> Blend {
    let f = THEME.lock().unwrap();
    let t = f.start.map_or(1.0, |s| (s.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0));
    Blend { from: f.from, to: f.to, t: t * t * (3.0 - 2.0 * t) }
}

/// Whether the colors are still flowing (the callers' frame timers run only then).
pub fn fading() -> bool {
    THEME.lock().unwrap().start.is_some_and(|s| s.elapsed() < FADE)
}

/// Sets or clears (`None`: the default look) the album theme. The new colors flow in; clearing is
/// instant. The caller repaints the title bar and the mini player.
pub fn set_theme(theme: Option<Theme>) {
    let mut f = THEME.lock().unwrap();
    if f.to == theme {
        return;
    }
    f.from = f.to;
    f.to = theme;
    f.start = theme.is_some().then(Instant::now);
}

pub fn lerp_color(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        (x + (y - x) * t).round() as u32
    };
    ch(16) << 16 | ch(8) << 8 | ch(0)
}

pub fn palette() -> Palette {
    let system_dark = dark_mode();
    let b = blend();
    let base = if system_dark { 0x202020 } else { 0xf3f3f3 };
    let (background, dark) = match b.to {
        Some(to) => (rgb(lerp_color(b.from.map_or(base, |f| f.ink), to.ink, b.t)), luminance(to.ink) < 0.5),
        None => (rgb(base), system_dark),
    };
    let text = if dark { rgb(0xffffff) } else { rgb(0x000000) };
    Palette { background, text, hover: with_alpha(text, 0.08), system_dark }
}

/// Relative luminance of an sRGB color, 0..1 (WCAG).
pub fn luminance(c: u32) -> f32 {
    let channel = |shift: u32| {
        let v = ((c >> shift) & 0xff) as f32 / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// The "app mode" of Settings > Personalization > Colors.
pub fn dark_mode() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result.is_ok() && value == 0
}

/// A window's Direct2D target, kept between paints and rebuilt when the device is lost.
#[derive(Default)]
pub struct Surface {
    target: Option<(ID2D1DCRenderTarget, ID2D1SolidColorBrush)>,
    /// Bumped whenever the target is rebuilt, so callers know to rebuild their bitmaps.
    pub generation: u32,
}

pub struct Canvas<'a> {
    pub target: &'a ID2D1DCRenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    /// `Surface::generation`: brushes made on an older target must be rebuilt.
    pub generation: u32,
}

impl Canvas<'_> {
    pub fn fill(&self, r: D2D_RECT_F, color: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&color);
            self.target.FillRectangle(&r, self.brush);
        }
    }

    pub fn pill(&self, r: D2D_RECT_F, radius: f32, color: D2D1_COLOR_F) {
        let rounded = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
        unsafe {
            self.brush.SetColor(&color);
            self.target.FillRoundedRectangle(&rounded, self.brush);
        }
    }

    /// A soft round glow brush: `color` at the center fading out at radius 1. Callers keep it between
    /// frames and move it with SetCenter/SetRadiusX/SetRadiusY/SetOpacity.
    pub fn radial(&self, color: D2D1_COLOR_F) -> Option<ID2D1RadialGradientBrush> {
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color },
            D2D1_GRADIENT_STOP { position: 0.45, color: with_alpha(color, 0.55) },
            D2D1_GRADIENT_STOP { position: 1.0, color: with_alpha(color, 0.0) },
        ];
        let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
            center: point(0.0, 0.0),
            gradientOriginOffset: point(0.0, 0.0),
            radiusX: 1.0,
            radiusY: 1.0,
        };
        unsafe {
            let stops =
                self.target.CreateGradientStopCollection(&stops, D2D1_GAMMA_1_0, D2D1_EXTEND_MODE_CLAMP).ok()?;
            self.target.CreateRadialGradientBrush(&props, None, &stops).ok()
        }
    }

    /// A gradient brush from `a` at `from` to `b` at `to`.
    pub fn linear(
        &self,
        from: Vector2,
        to: Vector2,
        a: D2D1_COLOR_F,
        b: D2D1_COLOR_F,
    ) -> Option<ID2D1LinearGradientBrush> {
        let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: a }, D2D1_GRADIENT_STOP { position: 1.0, color: b }];
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: from, endPoint: to };
        unsafe {
            let stops =
                self.target.CreateGradientStopCollection(&stops, D2D1_GAMMA_1_0, D2D1_EXTEND_MODE_CLAMP).ok()?;
            self.target.CreateLinearGradientBrush(&props, None, &stops).ok()
        }
    }

    pub fn solid(&self, color: D2D1_COLOR_F) -> Option<ID2D1SolidColorBrush> {
        unsafe { self.target.CreateSolidColorBrush(&color, None).ok() }
    }

    pub fn fill_with<B: windows_core::Param<ID2D1Brush>>(&self, r: D2D_RECT_F, brush: B) {
        unsafe { self.target.FillRectangle(&r, brush) }
    }

    /// A text layout drawn at `origin`; ranges without their own brush use `color`.
    pub fn layout(&self, layout: &IDWriteTextLayout, origin: Vector2, color: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&color);
            self.target.DrawTextLayout(origin, layout, self.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
    }

    pub fn text(&self, text: &str, format: &IDWriteTextFormat, r: D2D_RECT_F, color: D2D1_COLOR_F) {
        let text: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.brush.SetColor(&color);
            self.target.DrawText(
                &text,
                format,
                &r,
                self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
}

/// Glow brushes per theme, kept between frames: a fade draws the old and the new theme's glows at
/// partial opacity instead of rebuilding brushes every frame. Rebuilt with the render target.
#[derive(Default)]
pub struct GlowSets {
    generation: u32,
    sets: Vec<(Theme, Vec<ID2D1RadialGradientBrush>)>,
}

impl GlowSets {
    /// Keeps a brush set for each of `themes` (built once) and drops the others.
    pub fn prepare(&mut self, c: &Canvas, themes: &[Theme]) {
        if self.generation != c.generation {
            self.sets.clear();
            self.generation = c.generation;
        }
        self.sets.retain(|(t, _)| themes.contains(t));
        for theme in themes {
            if self.sets.iter().any(|(t, _)| t == theme) {
                continue;
            }
            if let Some(set) = theme.glows.iter().map(|g| c.radial(rgb(*g))).collect::<Option<Vec<_>>>() {
                self.sets.push((*theme, set));
            }
        }
    }

    pub fn get(&self, theme: &Theme) -> Option<&[ID2D1RadialGradientBrush]> {
        self.sets.iter().find(|(t, _)| t == theme).map(|(_, set)| set.as_slice())
    }
}

impl Surface {
    /// Paints the window: validates the update region and draws `area` (client pixels) with `draw`,
    /// in DIPs from the top left of the client area.
    pub fn paint(&mut self, hwnd: HWND, area: RECT, draw: impl FnOnce(&Canvas)) {
        let mut ps = PAINTSTRUCT::default();
        let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
        if let Err(e) = self.draw(hdc, hwnd, area, draw) {
            if e.code() == D2DERR_RECREATE_TARGET {
                self.target = None;
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            } else {
                eprintln!("paint failed: {e}");
            }
        }
        unsafe {
            let _ = EndPaint(hwnd, &ps);
        }
    }

    fn draw(
        &mut self,
        hdc: windows::Win32::Graphics::Gdi::HDC,
        hwnd: HWND,
        area: RECT,
        draw: impl FnOnce(&Canvas),
    ) -> Result<()> {
        if self.target.is_none() {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = unsafe { factories()?.d2d.CreateDCRenderTarget(&props)? };
            let brush = unsafe { target.CreateSolidColorBrush(&rgb(0), None)? };
            self.target = Some((target, brush));
            self.generation = self.generation.wrapping_add(1);
        }
        let (target, brush) = self.target.as_ref().expect("just set");
        let dpi = scale(hwnd) * 96.0;
        unsafe {
            target.BindDC(hdc, &area)?;
            target.SetDpi(dpi, dpi);
            target.BeginDraw();
            draw(&Canvas { target, brush, generation: self.generation });
            target.EndDraw(None, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_picks_readable_text() {
        assert!(luminance(0x000000) < 0.01);
        assert!(luminance(0xffffff) > 0.99);
        assert!(luminance(0x1db954) > 0.3);
    }
}
