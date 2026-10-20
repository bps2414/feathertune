//! Native title bar of the main window. The window has no system frame (`decorations(false)`; tao
//! keeps `WS_CAPTION`/`WS_SIZEBOX`, so snap, the min/max animations and the side and bottom resize
//! borders stay native). This module subclasses the window and draws a strip at the top of the
//! client area with Direct2D/DirectWrite (gfx.rs); the site is a child webview placed below it.
//! Nothing here is HTML, so the page gets no new permission.
//!
//! - WM_NCHITTEST: the top resize border comes from tao; on the strip the caption buttons answer
//!   HTMINBUTTON/HTMAXBUTTON/HTCLOSE (HTMAXBUTTON is what shows the Windows 11 snap layouts), the
//!   menu button is client area and the rest is HTCAPTION (drag, Aero Snap, double click maximizes,
//!   right click opens the system menu).
//! - The caption buttons are clicked on the non-client mouse messages, which are not passed on:
//!   DefWindowProc would draw the classic buttons over the strip.
//! - Sizes are DIPs scaled by the window's DPI; colors follow the system light/dark setting and the
//!   theme color (`gfx::set_theme_color` + `refresh`, for Phase 6).
//! - WM_SIZE moves the site webview below the strip right away, through its WebView2 controller.
//! - The look is "ink in water": soft color glows on deep ink under a glass sheen, laid out from the
//!   current track's id, so every track gets its own shape. A track change flows from the old shape to
//!   the new one (a 16 ms timer that runs only for `MORPH`); otherwise nothing animates.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Manager, PhysicalPosition, Webview, Window, Wry};
use tokio::sync::broadcast::error::RecvError;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, ICoreWebView2Controller,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_RANGE, IDWriteTextFormat, IDWriteTextLayout,
};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetClientRect, GetWindowLongPtrW, HTCAPTION, HTCLIENT, HTCLOSE, HTMAXBUTTON, HTMINBUTTON, IsZoomed,
    KillTimer, PostMessageW, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SIZE_MINIMIZED, SWP_NOACTIVATE, SWP_NOZORDER,
    SetTimer, SetWindowLongPtrW, SetWindowPos, WM_ACTIVATE, WM_APP, WM_CLOSE, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOVE, WM_NCACTIVATE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
    WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_PAINT, WM_SETFOCUS, WM_SETTINGCHANGE,
    WM_SIZE, WM_SYSCOMMAND, WM_TIMER, WS_CLIPCHILDREN,
};

use crate::gfx::{self, ICON_FONT, point, rect};
use crate::player::{Event, Player, State};

/// Strip height and button width, in DIPs (the Windows 11 caption metrics).
pub const HEIGHT: f32 = 32.0;
const BUTTON: f32 = 46.0;
const SUBCLASS_ID: usize = 0xfea7;
/// In the `Win32_UI_Controls` feature of `windows`, too big to enable for one constant.
pub const WM_MOUSELEAVE: u32 = 0x02a3;
/// Posted by the player thread when the track changed.
const WM_TRACK: u32 = WM_APP + 2;
const MORPH_TIMER: usize = 0xfea8;
const MORPH: Duration = Duration::from_millis(700);
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

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Menu,
    Minimize,
    Maximize,
    Close,
}

struct TitleBar {
    window: Window,
    controller: ICoreWebView2Controller,
    menu: Menu<Wry>,
    surface: gfx::Surface,
    name: IDWriteTextFormat,
    track: IDWriteTextFormat,
    icons: IDWriteTextFormat,
    now: Arc<Mutex<Now>>,
    /// The shape flows from `from` to `to` while `morph` runs.
    from: u64,
    to: u64,
    morph: Option<Instant>,
    active: bool,
    hover: Option<Button>,
    pressed: Option<Button>,
    /// Leave tracking requested, and whether for the non-client area.
    tracking: Option<bool>,
}

/// Subclasses the main window and lays the site webview out below the strip.
pub fn install(app: &AppHandle, window: &Window, site: &Webview) -> tauri::Result<()> {
    let hwnd = window.hwnd()?.0 as isize;
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?,
            &MenuItem::with_id(app, "mini_player", "Mini player", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )?;
    let window = window.clone();
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
        );
        let (Ok(name), Ok(track), Ok(icons)) = formats else {
            eprintln!("title bar: DirectWrite unavailable");
            return;
        };
        let bar = Box::new(TitleBar {
            window,
            controller: wv.controller(),
            menu,
            surface: gfx::Surface::default(),
            name,
            track,
            icons,
            now,
            from: seed,
            to: seed,
            morph: None,
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

/// Repaints the strip (after a theme color change).
#[allow(dead_code)] // with gfx::set_theme_color, for Phase 6
pub fn refresh(window: &Window) {
    if let Ok(hwnd) = window.hwnd() {
        invalidate(HWND(hwnd.0));
    }
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
        [Button::Menu, Button::Minimize, Button::Maximize, Button::Close]
            .into_iter()
            .find(|b| (button_left(*b, width)..button_left(*b, width) + BUTTON).contains(&x))
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
            Button::Menu => {
                let below = (HEIGHT * gfx::scale(hwnd)).round() as i32;
                if let Err(e) = self.window.popup_menu_at(&self.menu, PhysicalPosition::new(0, below)) {
                    eprintln!("title bar menu: {e}");
                }
            }
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
            unsafe {
                SetTimer(Some(hwnd), MORPH_TIMER, 16, None);
            }
        }
        invalidate(hwnd);
    }

    fn tick(&mut self, hwnd: HWND) {
        if self.morph.is_none_or(|start| start.elapsed() >= MORPH) {
            self.morph = None;
            unsafe {
                let _ = KillTimer(Some(hwnd), MORPH_TIMER);
            }
        }
        invalidate(hwnd);
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
        let themed = gfx::theme_color().is_some();
        let ink = if themed { pal.background } else { gfx::rgb(if dark { INK_DARK } else { INK_LIGHT }) };
        let full = if themed { pal.text } else { gfx::rgb(if dark { 0xf4f2ff } else { 0x1a1530 }) };
        let text = if self.active { full } else { gfx::with_alpha(full, 0.5) };
        let strength = (if dark { 1.0 } else { 0.5 }) * if self.active { 1.0 } else { 0.45 };
        // Smoothstep of the flow's progress.
        let t = self.morph.map_or(1.0, |start| (start.elapsed().as_secs_f32() / MORPH.as_secs_f32()).min(1.0));
        let t = t * t * (3.0 - 2.0 * t);
        let shape: Vec<Glow> = glows(self.from).into_iter().zip(glows(self.to)).map(|(a, b)| mix(a, b, t)).collect();
        let (hover, pressed) = (self.hover, self.pressed);
        let track = self.track_layout(width);
        let (name, icons) = (&self.name, &self.icons);
        self.surface.paint(hwnd, area, |c| {
            let strip = rect(0.0, 0.0, width, HEIGHT);
            c.fill(strip, ink);
            for (g, (color, alpha)) in shape.iter().zip(GLOWS) {
                let center = point(g.x * width, g.y * HEIGHT);
                c.glow(strip, center, g.rx * width, g.ry * HEIGHT, gfx::rgba(color, alpha * strength));
            }
            // Glass: a sheen on the upper half and a lit bottom edge.
            let sheen = if dark { 0.10 } else { 0.45 };
            c.vertical(rect(0.0, 0.0, width, HEIGHT * 0.55), gfx::rgba(0xffffff, sheen), gfx::rgba(0xffffff, 0.0));
            let edge = if dark { gfx::rgba(0xffffff, 0.09) } else { gfx::rgba(0x1a1530, 0.08) };
            c.fill(rect(0.0, HEIGHT - 1.0, width, HEIGHT), edge);
            for b in [Button::Menu, Button::Minimize, Button::Maximize, Button::Close] {
                let left = button_left(b, width);
                let r = rect(left, 0.0, left + BUTTON, HEIGHT);
                let mut glyph = text;
                if hover == Some(b) || pressed == Some(b) {
                    let down = pressed == Some(b);
                    let fill = match b {
                        Button::Close => gfx::rgba(CLOSE_HOVER, if down { 0.8 } else { 1.0 }),
                        _ => gfx::with_alpha(full, if down { 0.1 } else { 0.16 }),
                    };
                    c.pill(rect(left + 5.0, 4.0, left + BUTTON - 5.0, HEIGHT - 4.0), 7.0, fill);
                    if b == Button::Close {
                        glyph = gfx::rgb(0xffffff);
                    }
                }
                let icon = match b {
                    Button::Menu => "\u{e700}",
                    Button::Minimize => "\u{e921}",
                    Button::Maximize if maximized => "\u{e923}",
                    Button::Maximize => "\u{e922}",
                    Button::Close => "\u{e8bb}",
                };
                c.text(icon, icons, r, glyph);
            }
            c.text("feathertune", name, rect(BUTTON + 2.0, 0.0, BUTTON + 140.0, HEIGHT), text);
            if let Some((layout, left, artist)) = &track {
                // The artist is dimmed through a drawing effect: a brush of this target.
                if let Some(range) = artist
                    && let Ok(dim) = unsafe { c.target.CreateSolidColorBrush(&gfx::with_alpha(text, 0.6), None) }
                {
                    unsafe {
                        let _ = layout.SetDrawingEffect(&dim, *range);
                    }
                }
                c.layout(layout, point(*left, 0.0), text);
            }
        });
    }

    /// The current track, centered between the app name and the caption buttons: the title in
    /// semibold, then the artist (its range is returned, to be dimmed). None while nothing plays or
    /// the window is too narrow.
    fn track_layout(&self, width: f32) -> Option<(IDWriteTextLayout, f32, Option<DWRITE_TEXT_RANGE>)> {
        let (left, right) = (BUTTON + 150.0, button_left(Button::Minimize, width) - 24.0);
        let now = self.now.lock().unwrap();
        if now.title.is_empty() || right - left < 80.0 {
            return None;
        }
        let line = if now.artist.is_empty() { now.title.clone() } else { format!("{}    {}", now.title, now.artist) };
        let title_len = now.title.encode_utf16().count() as u32;
        let total = line.encode_utf16().count() as u32;
        let layout = gfx::text_layout(&line, &self.track, right - left, HEIGHT).ok()?;
        unsafe {
            let _ = layout
                .SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_RANGE { startPosition: 0, length: title_len });
        }
        let artist =
            (total > title_len).then_some(DWRITE_TEXT_RANGE { startPosition: title_len, length: total - title_len });
        Some((layout, left, artist))
    }
}

fn button_left(b: Button, width: f32) -> f32 {
    match b {
        Button::Menu => 0.0,
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
                Some(Button::Menu) => HTCLIENT,
                None => HTCAPTION,
            };
            return LRESULT(code as isize);
        }
        WM_NCMOUSEMOVE => bar.set_hover(hwnd, caption_button(wparam.0), true),
        WM_MOUSEMOVE => {
            let (x, y) = client_dip(hwnd, lparam);
            let hover = bar.hit(x, y, width_dip(hwnd)).filter(|b| *b == Button::Menu);
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
            if bar.hit(x, y, width_dip(hwnd)) == Some(Button::Menu) {
                if msg == WM_LBUTTONUP {
                    let clicked = bar.pressed == Some(Button::Menu);
                    bar.press(hwnd, None);
                    if clicked {
                        bar.click(hwnd, Button::Menu);
                    }
                } else {
                    bar.press(hwnd, Some(Button::Menu));
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
        WM_TIMER if wparam.0 == MORPH_TIMER => {
            bar.tick(hwnd);
            return LRESULT(0);
        }
        WM_SIZE => {
            if wparam.0 != SIZE_MINIMIZED as usize {
                bar.layout(hwnd, (lparam.0 & 0xffff) as i32, ((lparam.0 >> 16) & 0xffff) as i32);
            }
        }
        WM_MOVE => unsafe {
            let _ = bar.controller.NotifyParentWindowPositionChanged();
        },
        WM_NCACTIVATE => {
            bar.active = wparam.0 != 0;
            invalidate(hwnd);
        }
        WM_ACTIVATE | WM_SETTINGCHANGE => invalidate(hwnd),
        WM_SETFOCUS => {
            // The window got the focus itself (Alt+Tab, the taskbar): hand it to the page.
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            unsafe {
                let _ = bar.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
            return result;
        }
        WM_NCDESTROY => unsafe {
            let _ = KillTimer(Some(hwnd), MORPH_TIMER);
            let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
            drop(Box::from_raw(data as *mut TitleBar));
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
