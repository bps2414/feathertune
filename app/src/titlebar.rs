//! Native title bar of the main window. The window has no system frame (`decorations(false)`; tao
//! keeps `WS_CAPTION`/`WS_SIZEBOX`, so snap, the min/max animations and the side and bottom resize
//! borders stay native). This module subclasses the window and draws a strip at the top of the
//! client area with Direct2D/DirectWrite (gfx.rs); the site is a child webview placed below it.
//! Nothing here is HTML, so the page gets no new permission.
//!
//! - WM_NCHITTEST: the top resize border comes from tao; on the strip the caption buttons answer
//!   HTMINBUTTON/HTMAXBUTTON/HTCLOSE (HTMAXBUTTON is what shows the Windows 11 snap layouts), the
//!   share lyrics, mini player and settings buttons are client area and the rest is HTCAPTION (drag, Aero Snap,
//!   double click maximizes, right click opens the system menu). The share lyrics button is there only while the
//!   track has synced lyrics (`set_lyrics`, from lyrics.rs) and the `share_lyrics` plugin is on.
//! - The caption buttons are clicked on the non-client mouse messages, which are not passed on:
//!   DefWindowProc would draw the classic buttons over the strip.
//! - Sizes are DIPs scaled by the window's DPI; colors follow the system light/dark setting and, with
//!   the `album_color` plugin, the artwork's theme (`gfx::set_theme`, then `refresh`).
//! - WM_SIZE moves the site webview below the strip right away, through its WebView2 controller.
//! - The look is "ink in water": soft color glows on deep ink under a glass sheen, laid out from the
//!   current track's id, so every track gets its own shape. A track change flows from the old shape to
//!   the new one in `MORPH`; the glows' colors come from the artwork when `album_color` is on, and flow
//!   from the old theme to the new one the same way (the old and the new set of brushes are drawn at
//!   partial opacity, so nothing is rebuilt per frame).
//! - With the `animated_title_bar` plugin the glows also drift slowly. Performance rules: one 30 fps
//!   timer that runs only while there is something to move (a track change, or the drift while the
//!   window is visible, not minimized and in front; Windows animation effects do not stop it); the clock
//!   stops while paused, so it resumes where it was; only the strip is invalidated; brushes and the
//!   track's text layout are kept between frames and rebuilt only when the target, the theme, the
//!   track or the width change. A theme or shape flow also runs only while the window is on screen.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Webview, Window};
use tokio::sync::broadcast::error::RecvError;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, ICoreWebView2Controller,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::{ID2D1LinearGradientBrush, ID2D1SolidColorBrush};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_RANGE, IDWriteTextFormat, IDWriteTextLayout,
};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetClientRect, GetWindowLongPtrW, HTCAPTION, HTCLIENT, HTCLOSE, HTMAXBUTTON, HTMINBUTTON, IsIconic,
    IsWindowVisible, IsZoomed, KillTimer, PostMessageW, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SIZE_MINIMIZED,
    SWP_NOACTIVATE, SWP_NOZORDER, SetTimer, SetWindowLongPtrW, SetWindowPos, WM_ACTIVATE, WM_APP, WM_CLOSE,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOVE, WM_NCACTIVATE, WM_NCDESTROY, WM_NCHITTEST,
    WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_PAINT, WM_SETFOCUS,
    WM_SHOWWINDOW, WM_SIZE, WM_SYSCOMMAND, WM_TIMER, WS_CLIPCHILDREN,
};

use crate::gfx::{self, ICON_FONT, point, rect};
use crate::player::{Event, Player, State};
use crate::settings::Live;

/// Strip height and button width, in DIPs (the Windows 11 caption metrics).
pub const HEIGHT: f32 = 32.0;
const BUTTON: f32 = 46.0;
/// The mini player and settings buttons, left of the caption buttons with a small gap.
const SMALL: f32 = 40.0;
const GAP: f32 = 6.0;
/// Left margin of the app name.
const NAME_LEFT: f32 = 12.0;
const SUBCLASS_ID: usize = 0xfea7;
/// In the `Win32_UI_Controls` feature of `windows`, too big to enable for one constant.
pub const WM_MOUSELEAVE: u32 = 0x02a3;
/// Posted by the player thread when the track changed.
const WM_TRACK: u32 = WM_APP + 2;
/// Posted to re-check whether the animation timer should run.
const WM_ANIMATE: u32 = WM_APP + 3;
const TIMER: usize = 0xfea8;
/// 30 fps: smooth for slow drift, half the repaints of 60.
const FRAME_MS: u32 = 33;
const MORPH: Duration = Duration::from_millis(700);
/// The `animated_title_bar` plugin switch.
static ANIMATED: AtomicBool = AtomicBool::new(false);
/// The `share_lyrics` plugin was on at startup, and the current track has synced lyrics.
static SHARE_ON: AtomicBool = AtomicBool::new(false);
static LYRICS_READY: AtomicBool = AtomicBool::new(false);
/// The main window, for `set_lyrics`.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
const NAME_FONT: &str = "Segoe UI Variable Display";
const TRACK_FONT: &str = "Segoe UI Variable Text";

/// The ink below, then the four glows in drawing order with their strength (dark mode; light mode
/// softens them).
const INK_DARK: u32 = 0x120f24;
const INK_LIGHT: u32 = 0xf1eefb;
const GLOWS: [(u32, f32); 4] = [(0x6b3bf0, 0.9), (0xf0508a, 0.7), (0x22c3d6, 0.6), (0xf5a85c, 0.45)];
const CLOSE_HOVER: u32 = 0xe8435f;

/// The track the strip shows and the seed of its shape.
#[derive(Default)]
struct Now {
    seed: u64,
    title: String,
    artist: String,
}

impl Now {
    fn of(state: Option<&State>) -> Now {
        let Some(s) = state else { return Now::default() };
        let key = s.track.id.clone().unwrap_or_else(|| format!("{}\n{}", s.track.title, s.track.artist));
        // FNV-1a: stable across runs, so a track keeps its shape.
        let seed = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
        Now { seed, title: s.track.title.clone(), artist: s.track.artist.clone() }
    }
}

/// One glow: center and radii as fractions of the strip's width and height.
#[derive(Clone, Copy)]
struct Glow {
    x: f32,
    y: f32,
    rx: f32,
    ry: f32,
}

fn glows(seed: u64) -> [Glow; 4] {
    // splitmix64
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    };
    // Each color gets its own quarter of the width (rotated per track), so they never all pool on one side.
    let shift = (seed % 4) as usize;
    std::array::from_fn(|i| {
        let slot = ((i + shift) % 4) as f32;
        Glow {
            x: (slot + 0.15 + 0.7 * next()) / 4.0,
            y: -0.3 + 1.6 * next(),
            rx: 0.12 + 0.16 * next(),
            ry: 1.2 + next(),
        }
    })
}

fn mix(a: Glow, b: Glow, t: f32) -> Glow {
    let m = |p: f32, q: f32| p + (q - p) * t;
    Glow { x: m(a.x, b.x), y: m(a.y, b.y), rx: m(a.rx, b.rx), ry: m(a.ry, b.ry) }
}

/// The slow drift of glow `i` at `time` seconds: each glow sways and breathes at its own pace
/// (periods of 12 to 21 s), so the pattern never visibly repeats.
fn drift(g: Glow, i: usize, time: f32) -> Glow {
    const SPEED: [f32; 4] = [0.52, 0.41, 0.6, 0.3];
    let (w, phase) = (SPEED[i], i as f32 * 1.7);
    Glow {
        x: g.x + 0.05 * (time * w + phase).sin(),
        y: g.y + 0.35 * (time * w * 0.8 + phase * 1.3).sin(),
        rx: g.rx * (1.0 + 0.12 * (time * w * 1.3 + phase).sin()),
        ry: g.ry * (1.0 + 0.1 * (time * w * 0.7 + phase).cos()),
    }
}

/// Glass and text brushes, kept between frames and rebuilt with the render target or on a light/dark
/// change.
struct Brushes {
    generation: u32,
    dark: bool,
    sheen: ID2D1LinearGradientBrush,
    /// The artist's color in the track line (a drawing effect of its layout).
    dim: ID2D1SolidColorBrush,
}

impl Brushes {
    fn new(c: &gfx::Canvas, dark: bool) -> Option<Brushes> {
        let sheen = if dark { 0.10 } else { 0.45 };
        let sheen =
            c.linear(point(0.0, 0.0), point(0.0, HEIGHT * 0.55), gfx::rgba(0xffffff, sheen), gfx::rgba(0xffffff, 0.0))?;
        Some(Brushes { generation: c.generation, dark, sheen, dim: c.solid(gfx::rgb(0))? })
    }
}

/// The current track line, kept until the track, the width or the brushes change.
struct TrackLine {
    key: (String, String, u32, u32),
    layout: IDWriteTextLayout,
    left: f32,
}

impl TrackLine {
    /// The title in semibold, then the artist dimmed, centered between the app name and the caption
    /// buttons. None while nothing plays or the window is too narrow.
    fn new(
        format: &IDWriteTextFormat,
        title: &str,
        artist: &str,
        width: f32,
        dim: &ID2D1SolidColorBrush,
    ) -> Option<Self> {
        let (left, right) = (NAME_LEFT + 150.0, first_button_left(width) - 24.0);
        if title.is_empty() || right - left < 80.0 {
            return None;
        }
        let line = if artist.is_empty() { title.to_string() } else { format!("{title}    {artist}") };
        let title_len = title.encode_utf16().count() as u32;
        let total = line.encode_utf16().count() as u32;
        let layout = gfx::text_layout(&line, format, right - left, HEIGHT).ok()?;
        unsafe {
            let _ = layout
                .SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_RANGE { startPosition: 0, length: title_len });
            if total > title_len {
                let _ = layout
                    .SetDrawingEffect(dim, DWRITE_TEXT_RANGE { startPosition: title_len, length: total - title_len });
            }
        }
        Some(TrackLine { key: Default::default(), layout, left })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Share,
    MiniPlayer,
    Settings,
    Minimize,
    Maximize,
    Close,
}

struct TitleBar {
    window: Window,
    controller: ICoreWebView2Controller,
    surface: gfx::Surface,
    name: IDWriteTextFormat,
    track: IDWriteTextFormat,
    icons: IDWriteTextFormat,
    /// Our buttons' glyphs: larger than the caption glyphs, which keep the Windows size.
    big_icons: IDWriteTextFormat,
    now: Arc<Mutex<Now>>,
    /// The shape flows from `from` to `to` while `morph` runs.
    from: u64,
    to: u64,
    morph: Option<Instant>,
    /// Seconds of drift so far; advances only while the timer runs.
    time: f32,
    last_frame: Option<Instant>,
    ticking: bool,
    brushes: Option<Brushes>,
    /// The glow brushes of the themes on screen.
    glows: gfx::GlowSets,
    /// Bumped when `brushes` is rebuilt (the track line holds one of them).
    brushes_id: u32,
    line: Option<TrackLine>,
    active: bool,
    hover: Option<Button>,
    pressed: Option<Button>,
    /// Leave tracking requested, and whether for the non-client area.
    tracking: Option<bool>,
}

/// Subclasses the main window and lays the site webview out below the strip.
pub fn install(app: &AppHandle, window: &Window, site: &Webview) -> tauri::Result<()> {
    let hwnd = window.hwnd()?.0 as isize;
    let window = window.clone();
    ANIMATED.store(app.state::<Live>().plugin("animated_title_bar"), Ordering::Relaxed);
    SHARE_ON.store(app.state::<Live>().plugin("share_lyrics"), Ordering::Relaxed);
    WINDOW.store(hwnd, Ordering::Relaxed);
    let player = app.state::<Player>();
    let now = Arc::new(Mutex::new(Now::of(player.current().as_ref())));
    let seed = now.lock().unwrap().seed;
    let (shared, mut rx) = (now.clone(), player.subscribe());
    std::thread::spawn(move || {
        loop {
            match rx.blocking_recv() {
                Ok(Event::Track(state)) => {
                    *shared.lock().unwrap() = Now::of(Some(&state));
                    unsafe {
                        let _ = PostMessageW(Some(HWND(hwnd as _)), WM_TRACK, WPARAM(0), LPARAM(0));
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
    site.with_webview(move |wv| {
        let hwnd = HWND(hwnd as _);
        let formats = (
            gfx::text_format(NAME_FONT, 12.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING),
            gfx::text_format(TRACK_FONT, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER),
            gfx::text_format(ICON_FONT, 10.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER),
            gfx::text_format(ICON_FONT, 14.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER),
        );
        let (Ok(name), Ok(track), Ok(icons), Ok(big_icons)) = formats else {
            eprintln!("title bar: DirectWrite unavailable");
            return;
        };
        let bar = Box::new(TitleBar {
            window,
            controller: wv.controller(),
            surface: gfx::Surface::default(),
            name,
            track,
            icons,
            big_icons,
            now,
            from: seed,
            to: seed,
            morph: None,
            time: 0.0,
            last_frame: None,
            ticking: false,
            brushes: None,
            glows: gfx::GlowSets::default(),
            brushes_id: 0,
            line: None,
            active: true,
            hover: None,
            pressed: None,
            tracking: None,
        });
        unsafe {
            // The strip's paint must never cover the webview.
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_CLIPCHILDREN.0 as isize);
            let bar = Box::into_raw(bar);
            if !SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, bar as usize).as_bool() {
                eprintln!("title bar: SetWindowSubclass failed");
                drop(Box::from_raw(bar));
                return;
            }
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            (*bar).layout(hwnd, client.right, client.bottom);
        }
    })
}

/// Turns the drift on or off (the `animated_title_bar` switch).
pub fn set_animated(app: &AppHandle, on: bool) {
    ANIMATED.store(on, Ordering::Relaxed);
    if let Some(window) = app.get_window("main")
        && let Ok(hwnd) = window.hwnd()
    {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd.0)), WM_ANIMATE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Shows or hides the share lyrics button: whether the current track has synced lyrics. Any thread.
pub fn set_lyrics(ready: bool) {
    if LYRICS_READY.swap(ready, Ordering::Relaxed) != ready {
        let hwnd = WINDOW.load(Ordering::Relaxed);
        if hwnd != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as _)), WM_ANIMATE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Repaints the strip and starts the flow to a new theme (after `gfx::set_theme`). Any thread.
pub fn refresh(app: &AppHandle) {
    set_animated(app, ANIMATED.load(Ordering::Relaxed));
}

fn invalidate(hwnd: HWND) {
    let height = (HEIGHT * gfx::scale(hwnd)).round() as i32;
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        let strip = RECT { bottom: height, ..client };
        let _ = InvalidateRect(Some(hwnd), Some(&strip), false);
    }
}

impl TitleBar {
    /// The site webview takes the client area below the strip.
    fn layout(&self, hwnd: HWND, width: i32, height: i32) {
        let top = ((HEIGHT * gfx::scale(hwnd)).round() as i32).min(height);
        unsafe {
            let _ = self.controller.SetBounds(RECT { left: 0, top: 0, right: width, bottom: height - top });
            let mut container = HWND::default();
            if self.controller.ParentWindow(&mut container).is_ok() {
                let _ = SetWindowPos(container, None, 0, top, width, height - top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        invalidate(hwnd);
    }

    /// The button under a point of the strip, in DIPs.
    fn hit(&self, x: f32, y: f32, width: f32) -> Option<Button> {
        if !(0.0..HEIGHT).contains(&y) {
            return None;
        }
        BUTTONS
            .into_iter()
            .filter(|b| shown(*b))
            .find(|b| (button_left(*b, width)..button_left(*b, width) + button_width(*b)).contains(&x))
    }

    fn set_hover(&mut self, hwnd: HWND, hover: Option<Button>, nonclient: bool) {
        if self.tracking != Some(nonclient) {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: if nonclient { TME_LEAVE | TME_NONCLIENT } else { TME_LEAVE },
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            if unsafe { TrackMouseEvent(&mut tme) }.is_ok() {
                self.tracking = Some(nonclient);
            }
        }
        if self.hover != hover {
            self.hover = hover;
            invalidate(hwnd);
        }
    }

    fn press(&mut self, hwnd: HWND, button: Option<Button>) {
        if self.pressed != button {
            self.pressed = button;
            invalidate(hwnd);
        }
    }

    fn click(&mut self, hwnd: HWND, button: Button) {
        let command = |c: u32| unsafe {
            let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(c as usize), LPARAM(0));
        };
        match button {
            Button::Share => crate::share_lyrics::open(self.window.app_handle()),
            Button::MiniPlayer => crate::miniplayer::toggle(self.window.app_handle()),
            Button::Settings => crate::settings_window::open(self.window.app_handle()),
            Button::Minimize => command(SC_MINIMIZE),
            Button::Maximize if unsafe { IsZoomed(hwnd) }.as_bool() => command(SC_RESTORE),
            Button::Maximize => command(SC_MAXIMIZE),
            // The app's close: hide to the tray (main.rs).
            Button::Close => unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
        }
    }

    /// A new track: start flowing to its shape.
    fn track_changed(&mut self, hwnd: HWND) {
        let seed = self.now.lock().unwrap().seed;
        if seed != self.to {
            self.from = self.to;
            self.to = seed;
            self.morph = Some(Instant::now());
        }
        self.schedule(hwnd);
        invalidate(hwnd);
    }

    /// Starts or stops the frame timer: it runs only while there is something to move on screen.
    fn schedule(&mut self, hwnd: HWND) {
        let on_screen = unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() };
        let drifting = ANIMATED.load(Ordering::Relaxed) && self.active && on_screen;
        let run = ((self.morph.is_some() || gfx::fading()) && on_screen) || drifting;
        if run == self.ticking {
            return;
        }
        self.ticking = run;
        unsafe {
            if run {
                SetTimer(Some(hwnd), TIMER, FRAME_MS, None);
                self.last_frame = Some(Instant::now());
            } else {
                let _ = KillTimer(Some(hwnd), TIMER);
                self.last_frame = None;
            }
        }
    }

    fn tick(&mut self, hwnd: HWND) {
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            // Capped, so a stalled message loop does not jump the drift.
            self.time += (now - last).as_secs_f32().min(0.1);
        }
        self.last_frame = Some(now);
        if self.morph.is_some_and(|start| start.elapsed() >= MORPH) {
            self.morph = None;
        }
        invalidate(hwnd);
        self.schedule(hwnd);
    }

    fn paint(&mut self, hwnd: HWND) {
        let scale = gfx::scale(hwnd);
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        let area = RECT { bottom: (HEIGHT * scale).round() as i32, ..client };
        let width = client.right as f32 / scale;
        let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
        let pal = gfx::palette();
        let dark = pal.text.r > 0.5;
        let themes = gfx::blend();
        let themed = themes.to.is_some();
        // The default look, for the side of a flow that has no album theme.
        let default = gfx::Theme {
            ink: if pal.system_dark { INK_DARK } else { INK_LIGHT },
            glows: GLOWS.map(|(color, _)| color),
        };
        let (from, to) = (themes.from.unwrap_or(default), themes.to.unwrap_or(default));
        let ink = gfx::rgb(gfx::lerp_color(from.ink, to.ink, themes.t));
        let full = if themed { pal.text } else { gfx::rgb(if dark { 0xf4f2ff } else { 0x1a1530 }) };
        let text = if self.active { full } else { gfx::with_alpha(full, 0.5) };
        let strength = (if dark { 1.0 } else { 0.5 }) * if self.active { 1.0 } else { 0.45 };
        // Smoothstep of the shape flow's progress.
        let t = self.morph.map_or(1.0, |start| (start.elapsed().as_secs_f32() / MORPH.as_secs_f32()).min(1.0));
        let t = t * t * (3.0 - 2.0 * t);
        let drifting = ANIMATED.load(Ordering::Relaxed);
        let time = self.time;
        let shape: Vec<Glow> = glows(self.from)
            .into_iter()
            .zip(glows(self.to))
            .enumerate()
            .map(|(i, (a, b))| if drifting { drift(mix(a, b, t), i, time) } else { mix(a, b, t) })
            .collect();
        let (hover, pressed) = (self.hover, self.pressed);
        let (title, artist) = {
            let now = self.now.lock().unwrap();
            (now.title.clone(), now.artist.clone())
        };
        let (name, icons, big_icons, track) = (&self.name, &self.icons, &self.big_icons, &self.track);
        let (brushes, brushes_id, line) = (&mut self.brushes, &mut self.brushes_id, &mut self.line);
        let sets = &mut self.glows;
        // The theme flow draws both sets of glows, each at its share of the opacity.
        let layers = if from == to { vec![(to, 1.0)] } else { vec![(from, 1.0 - themes.t), (to, themes.t)] };
        self.surface.paint(hwnd, area, |c| {
            let strip = rect(0.0, 0.0, width, HEIGHT);
            c.fill(strip, ink);
            if brushes.as_ref().is_none_or(|b| b.generation != c.generation || b.dark != dark) {
                *brushes = Brushes::new(c, dark);
                *brushes_id = brushes_id.wrapping_add(1);
            }
            let Some(b) = brushes.as_ref() else { return };
            sets.prepare(c, &[from, to]);
            for (theme, share) in &layers {
                let Some(set) = sets.get(theme) else { continue };
                for ((g, (_, alpha)), brush) in shape.iter().zip(GLOWS).zip(set) {
                    unsafe {
                        brush.SetCenter(point(g.x * width, g.y * HEIGHT));
                        brush.SetRadiusX(g.rx * width);
                        brush.SetRadiusY(g.ry * HEIGHT);
                        brush.SetOpacity(alpha * strength * share);
                    }
                    c.fill_with(strip, brush);
                }
            }
            // Glass: a sheen on the upper half and a lit bottom edge.
            c.fill_with(rect(0.0, 0.0, width, HEIGHT * 0.55), &b.sheen);
            let edge = if dark { gfx::rgba(0xffffff, 0.09) } else { gfx::rgba(0x1a1530, 0.08) };
            c.fill(rect(0.0, HEIGHT - 1.0, width, HEIGHT), edge);
            for button in BUTTONS.into_iter().filter(|b| shown(*b)) {
                let left = button_left(button, width);
                let right = left + button_width(button);
                let r = rect(left, 0.0, right, HEIGHT);
                let mut glyph = text;
                if hover == Some(button) || pressed == Some(button) {
                    let down = pressed == Some(button);
                    let fill = match button {
                        Button::Close => gfx::rgba(CLOSE_HOVER, if down { 0.8 } else { 1.0 }),
                        _ => gfx::with_alpha(full, if down { 0.1 } else { 0.16 }),
                    };
                    c.pill(rect(left + 5.0, 4.0, right - 5.0, HEIGHT - 4.0), 7.0, fill);
                    if button == Button::Close {
                        glyph = gfx::rgb(0xffffff);
                    }
                }
                let icon = match button {
                    Button::Share => "\u{e72d}",
                    Button::MiniPlayer => "\u{e8a7}",
                    Button::Settings => "\u{e713}",
                    Button::Minimize => "\u{e921}",
                    Button::Maximize if maximized => "\u{e923}",
                    Button::Maximize => "\u{e922}",
                    Button::Close => "\u{e8bb}",
                };
                c.text(icon, if ours(button) { big_icons } else { icons }, r, glyph);
            }
            c.text("feathertune", name, rect(NAME_LEFT, 0.0, NAME_LEFT + 140.0, HEIGHT), text);
            let key = (title, artist, first_button_left(width).to_bits(), *brushes_id);
            if line.as_ref().is_none_or(|l| l.key != key) {
                *line = TrackLine::new(track, &key.0, &key.1, width, &b.dim).map(|l| TrackLine { key, ..l });
            }
            if let Some(l) = line.as_ref() {
                unsafe { b.dim.SetColor(&gfx::with_alpha(text, 0.6)) };
                c.layout(&l.layout, point(l.left, 0.0), text);
            }
        });
    }
}

const BUTTONS: [Button; 6] =
    [Button::Share, Button::MiniPlayer, Button::Settings, Button::Minimize, Button::Maximize, Button::Close];

/// The buttons that are client area (ours), not caption buttons.
fn ours(b: Button) -> bool {
    matches!(b, Button::Share | Button::MiniPlayer | Button::Settings)
}

fn shown(b: Button) -> bool {
    b != Button::Share || (SHARE_ON.load(Ordering::Relaxed) && LYRICS_READY.load(Ordering::Relaxed))
}

/// Where the leftmost button on the strip starts.
fn first_button_left(width: f32) -> f32 {
    BUTTONS.into_iter().filter(|b| shown(*b)).map(|b| button_left(b, width)).fold(width, f32::min)
}

fn button_width(b: Button) -> f32 {
    if ours(b) { SMALL } else { BUTTON }
}

fn button_left(b: Button, width: f32) -> f32 {
    match b {
        Button::Share => width - 3.0 * BUTTON - GAP - 3.0 * SMALL,
        Button::MiniPlayer => width - 3.0 * BUTTON - GAP - 2.0 * SMALL,
        Button::Settings => width - 3.0 * BUTTON - GAP - SMALL,
        Button::Minimize => width - 3.0 * BUTTON,
        Button::Maximize => width - 2.0 * BUTTON,
        Button::Close => width - BUTTON,
    }
}

fn caption_button(hit: usize) -> Option<Button> {
    match hit as u32 {
        HTMINBUTTON => Some(Button::Minimize),
        HTMAXBUTTON => Some(Button::Maximize),
        HTCLOSE => Some(Button::Close),
        _ => None,
    }
}

/// Client point of a mouse message, in DIPs.
fn client_dip(hwnd: HWND, lparam: LPARAM) -> (f32, f32) {
    let scale = gfx::scale(hwnd);
    let (x, y) = (lparam.0 as i16 as f32, (lparam.0 >> 16) as i16 as f32);
    (x / scale, y / scale)
}

fn width_dip(hwnd: HWND) -> f32 {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    client.right as f32 / gfx::scale(hwnd)
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let bar = unsafe { &mut *(data as *mut TitleBar) };
    match msg {
        WM_NCHITTEST => {
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            if result.0 as u32 != HTCLIENT {
                return result;
            }
            let mut p = POINT { x: lparam.0 as i16 as i32, y: (lparam.0 >> 16) as i16 as i32 };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut p);
            }
            let scale = gfx::scale(hwnd);
            let (x, y) = (p.x as f32 / scale, p.y as f32 / scale);
            if y >= HEIGHT {
                return result;
            }
            let code = match bar.hit(x, y, width_dip(hwnd)) {
                Some(Button::Minimize) => HTMINBUTTON,
                Some(Button::Maximize) => HTMAXBUTTON,
                Some(Button::Close) => HTCLOSE,
                Some(_) => HTCLIENT,
                None => HTCAPTION,
            };
            return LRESULT(code as isize);
        }
        WM_NCMOUSEMOVE => bar.set_hover(hwnd, caption_button(wparam.0), true),
        WM_MOUSEMOVE => {
            let (x, y) = client_dip(hwnd, lparam);
            let hover = bar.hit(x, y, width_dip(hwnd)).filter(|b| ours(*b));
            bar.set_hover(hwnd, hover, false);
        }
        WM_NCMOUSELEAVE | WM_MOUSELEAVE => {
            bar.tracking = None;
            if bar.hover.take().is_some() | bar.pressed.take().is_some() {
                invalidate(hwnd);
            }
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
            if let Some(b) = caption_button(wparam.0) {
                bar.press(hwnd, Some(b));
                return LRESULT(0);
            }
        }
        WM_NCLBUTTONUP => {
            if let Some(b) = caption_button(wparam.0) {
                let clicked = bar.pressed == Some(b);
                bar.press(hwnd, None);
                if clicked {
                    bar.click(hwnd, b);
                }
                return LRESULT(0);
            }
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP => {
            let (x, y) = client_dip(hwnd, lparam);
            if let Some(b) = bar.hit(x, y, width_dip(hwnd)).filter(|b| ours(*b)) {
                if msg == WM_LBUTTONUP {
                    let clicked = bar.pressed == Some(b);
                    bar.press(hwnd, None);
                    if clicked {
                        bar.click(hwnd, b);
                    }
                } else {
                    bar.press(hwnd, Some(b));
                }
                return LRESULT(0);
            }
        }
        WM_PAINT => {
            bar.paint(hwnd);
            return LRESULT(0);
        }
        WM_TRACK => {
            bar.track_changed(hwnd);
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == TIMER => {
            bar.tick(hwnd);
            return LRESULT(0);
        }
        WM_ANIMATE => {
            bar.schedule(hwnd);
            invalidate(hwnd);
            return LRESULT(0);
        }
        // Shown or hidden (the tray): IsWindowVisible changes after this message, so check later.
        WM_SHOWWINDOW => unsafe {
            let _ = PostMessageW(Some(hwnd), WM_ANIMATE, WPARAM(0), LPARAM(0));
        },
        WM_SIZE => {
            if wparam.0 != SIZE_MINIMIZED as usize {
                bar.layout(hwnd, (lparam.0 & 0xffff) as i32, ((lparam.0 >> 16) & 0xffff) as i32);
            }
            bar.schedule(hwnd);
        }
        WM_MOVE => unsafe {
            let _ = bar.controller.NotifyParentWindowPositionChanged();
        },
        WM_NCACTIVATE => {
            bar.active = wparam.0 != 0;
            bar.schedule(hwnd);
            invalidate(hwnd);
        }
        WM_ACTIVATE => invalidate(hwnd),
        WM_SETFOCUS => {
            // The window got the focus itself (Alt+Tab, the taskbar): hand it to the page.
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            unsafe {
                let _ = bar.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
            return result;
        }
        WM_NCDESTROY => unsafe {
            let _ = KillTimer(Some(hwnd), TIMER);
            let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
            drop(Box::from_raw(data as *mut TitleBar));
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
