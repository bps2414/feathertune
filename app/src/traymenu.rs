//! The tray menu in the app's look (plugin `themed_tray_menu`): a native popup drawn with Direct2D
//! like the mini player (the album ink and glows, or the system colors), with the track and
//! previous / play-pause / next on top and the tray items below. Without the plugin the tray keeps the
//! Windows menu (main.rs).
//!
//! - Opened by a right click on the tray icon, at the cursor, inside the work area of its monitor;
//!   sized for that monitor's DPI. Its items and their states are read when it opens.
//! - It closes when it loses the focus, on Esc, on a click outside or after an item runs. If Windows
//!   does not give it the foreground, it captures the mouse so a click outside still closes it.
//! - Keyboard: Up/Down move between the items, Enter or Space runs one.
//! - It exists only while open, and repaints only on hover and focus changes (no timer). Screen
//!   readers do not see it as a menu, unlike the Windows one: turn the plugin off for that.

use std::sync::Once;
use std::sync::atomic::{AtomicIsize, Ordering};

use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE,
    VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetClientRect, GetCursorPos, GetWindowLongPtrW, IDC_ARROW, LoadCursorW, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, WA_INACTIVE, WM_ACTIVATE,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Result, w};

use crate::gfx::{self, ICON_FONT, TEXT_FONT, point, rect};
use crate::player::Player;
use crate::settings::Live;
use crate::titlebar::WM_MOUSELEAVE;

/// Sizes in DIPs.
const WIDTH: f32 = 272.0;
const PAD: f32 = 6.0;
const HEADER: f32 = 62.0;
const ITEM: f32 = 32.0;
const SEPARATOR: f32 = 9.0;
const CONTROL: f32 = 34.0;
/// The static glows behind the header: center, radii and strength (like the mini player's).
const GLOWS: [(f32, f32, f32, f32, f32); 2] = [(WIDTH, 0.0, 200.0, 110.0, 0.5), (40.0, 0.0, 160.0, 80.0, 0.35)];

/// The open menu, 0 when closed. Main thread only.
static WINDOW: AtomicIsize = AtomicIsize::new(0);

enum Entry {
    Item { id: &'static str, label: String, checked: Option<bool>, enabled: bool },
    Separator,
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
    /// previous, play/pause, next
    Control(usize),
    Item(usize),
}

const CONTROLS: [&str; 3] = ["previous", "play_pause", "next"];

struct Menu {
    app: AppHandle,
    entries: Vec<Entry>,
    title: String,
    artist: String,
    playing: bool,
    has_track: bool,
    surface: gfx::Surface,
    glows: gfx::GlowSets,
    name: IDWriteTextFormat,
    small: IDWriteTextFormat,
    label: IDWriteTextFormat,
    icons: IDWriteTextFormat,
    hover: Option<Target>,
    pressed: Option<Target>,
    /// Item focused with the keyboard.
    focus: Option<usize>,
    tracking: bool,
    /// The mouse is captured (no foreground): a click outside closes it.
    captured: bool,
    /// DestroyWindow was called (it sends WM_ACTIVATE on the way).
    closing: bool,
}

/// Opens the menu at the cursor (any thread).
pub fn open(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let open = WINDOW.load(Ordering::Relaxed);
        if open != 0 {
            let _ = unsafe { DestroyWindow(HWND(open as _)) };
        }
        if let Err(e) = unsafe { create(&handle) } {
            eprintln!("tray menu unavailable: {e}");
        }
    });
}

/// The items and their states, now.
fn entries(app: &AppHandle) -> Vec<Entry> {
    let item = |id, label: &str, checked, enabled| Entry::Item { id, label: label.into(), checked, enabled };
    let shown = app.get_window("main").is_some_and(|w| w.is_visible().unwrap_or(false));
    let live = app.state::<Live>();
    let lyrics = live.plugin("desktop_lyrics");
    let lyrics_open = crate::desktop_lyrics::is_open();
    let lastfm = app.try_state::<crate::lastfm::LastFm>().map(|l| l.connected());
    let mut list = vec![
        item("toggle", if shown { "Hide window" } else { "Show window" }, None, true),
        item("mini_player", "Mini player", Some(crate::miniplayer::is_open()), live.plugin("mini_player")),
        item("desktop_lyrics", "Floating lyrics", Some(lyrics_open), lyrics),
    ];
    if lyrics_open {
        let label = if crate::desktop_lyrics::is_locked() { "Unlock floating lyrics" } else { "Lock floating lyrics" };
        list.push(item("desktop_lyrics_lock", label, None, true));
    }
    list.push(item("share_lyrics", "Share lyrics...", None, live.plugin("share_lyrics")));
    list.extend([
        Entry::Separator,
        item("autostart", "Start with Windows", Some(app.autolaunch().is_enabled().unwrap_or(false)), true),
        item("lastfm", crate::lastfm_label(lastfm), None, lastfm.is_some()),
        item("settings", "Settings", None, true),
        Entry::Separator,
        item("quit", "Quit", None, true),
    ]);
    list
}

fn height(entries: &[Entry]) -> f32 {
    HEADER
        + SEPARATOR
        + entries.iter().map(|e| if matches!(e, Entry::Separator) { SEPARATOR } else { ITEM }).sum::<f32>()
        + PAD
}

fn control_rect(i: usize) -> D2D_RECT_F {
    let right = WIDTH - 10.0 - (2 - i) as f32 * (CONTROL + 2.0);
    rect(right - CONTROL, 14.0, right, 14.0 + CONTROL)
}

/// The band of every entry, in order.
fn entry_rects(entries: &[Entry]) -> Vec<D2D_RECT_F> {
    let mut y = HEADER + SEPARATOR;
    entries
        .iter()
        .map(|e| {
            let h = if matches!(e, Entry::Separator) { SEPARATOR } else { ITEM };
            y += h;
            rect(PAD, y - h, WIDTH - PAD, y)
        })
        .collect()
}

impl Menu {
    fn hit(&self, x: f32, y: f32) -> Option<Target> {
        let inside = |r: D2D_RECT_F| (r.left..r.right).contains(&x) && (r.top..r.bottom).contains(&y);
        if let Some(i) = (0..3).find(|i| inside(control_rect(*i))) {
            return self.has_track.then_some(Target::Control(i));
        }
        let rects = entry_rects(&self.entries);
        (0..rects.len())
            .find(|i| matches!(self.entries[*i], Entry::Item { enabled: true, .. }) && inside(rects[*i]))
            .map(Target::Item)
    }

    /// The next enabled item from `from` in `step` (±1), wrapping.
    fn step_focus(&self, step: isize) -> Option<usize> {
        let n = self.entries.len() as isize;
        let start = self.focus.map_or(if step > 0 { -1 } else { n }, |f| f as isize);
        (1..=n)
            .map(|k| (start + step * k).rem_euclid(n) as usize)
            .find(|i| matches!(self.entries[*i], Entry::Item { enabled: true, .. }))
    }

    fn paint(&mut self, hwnd: HWND) {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        let pal = gfx::palette();
        let theme = gfx::blend().to;
        let total = height(&self.entries);
        let rects = entry_rects(&self.entries);
        let glows = &mut self.glows;
        let (hover, pressed, focus) = (self.hover, self.pressed, self.focus);
        let (entries, title, artist) = (&self.entries, &self.title, &self.artist);
        let (name, small, label, icons) = (&self.name, &self.small, &self.label, &self.icons);
        let (has_track, playing) = (self.has_track, self.playing);
        self.surface.paint(hwnd, client, |c| {
            c.fill(rect(0.0, 0.0, WIDTH, total), pal.background);
            if let Some(theme) = theme {
                glows.prepare(c, &[theme]);
                for (brush, (x, y, rx, ry, alpha)) in glows.get(&theme).unwrap_or_default().iter().zip(GLOWS) {
                    unsafe {
                        brush.SetCenter(point(x, y));
                        brush.SetRadiusX(rx);
                        brush.SetRadiusY(ry);
                        brush.SetOpacity(alpha);
                    }
                    c.fill_with(rect(0.0, 0.0, WIDTH, HEADER), brush);
                }
            }
            let text_right = control_rect(0).left - 8.0;
            if has_track {
                c.text(title, name, rect(14.0, 12.0, text_right, 32.0), pal.text);
                c.text(artist, small, rect(14.0, 32.0, text_right, 50.0), gfx::with_alpha(pal.text, 0.7));
            } else {
                c.text("Nothing playing", name, rect(14.0, 12.0, text_right, 50.0), gfx::with_alpha(pal.text, 0.7));
            }
            for (i, glyph) in ["\u{e892}", if playing { "\u{e769}" } else { "\u{e768}" }, "\u{e893}"].iter().enumerate()
            {
                let r = control_rect(i);
                let t = Target::Control(i);
                let mut color = gfx::with_alpha(pal.text, if has_track { 1.0 } else { 0.35 });
                if i == 1 {
                    // Play/pause is a solid disc with the glyph cut in the background color, as in the mini player.
                    let alpha = if pressed == Some(t) {
                        0.75
                    } else if hover == Some(t) {
                        1.0
                    } else {
                        0.9
                    };
                    c.pill(r, CONTROL / 2.0, gfx::with_alpha(pal.text, alpha * if has_track { 1.0 } else { 0.35 }));
                    color = pal.background;
                } else if hover == Some(t) || pressed == Some(t) {
                    c.pill(r, 8.0, gfx::with_alpha(pal.text, if pressed == Some(t) { 0.1 } else { 0.16 }));
                }
                c.text(glyph, icons, r, color);
            }
            c.fill(rect(PAD + 8.0, HEADER + 4.0, WIDTH - PAD - 8.0, HEADER + 5.0), gfx::with_alpha(pal.text, 0.1));
            for (i, (entry, r)) in entries.iter().zip(&rects).enumerate() {
                match entry {
                    Entry::Separator => {
                        let mid = (r.top + r.bottom) / 2.0;
                        c.fill(rect(r.left + 8.0, mid, r.right - 8.0, mid + 1.0), gfx::with_alpha(pal.text, 0.1));
                    }
                    Entry::Item { label: text, checked, enabled, .. } => {
                        let t = Target::Item(i);
                        if *enabled && (hover == Some(t) || pressed == Some(t) || focus == Some(i)) {
                            c.pill(*r, 6.0, gfx::with_alpha(pal.text, if pressed == Some(t) { 0.08 } else { 0.12 }));
                        }
                        let color = gfx::with_alpha(pal.text, if *enabled { 1.0 } else { 0.4 });
                        if *checked == Some(true) {
                            c.text("\u{e73e}", icons, rect(r.left, r.top, r.left + 30.0, r.bottom), color);
                        }
                        c.text(text, label, rect(r.left + 32.0, r.top, r.right - 10.0, r.bottom), color);
                    }
                }
            }
        });
    }

    /// Destroys the window, which frees `self`: call it last.
    fn close(&mut self, hwnd: HWND) {
        if !self.closing {
            self.closing = true;
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    /// Runs a control or an item, after closing the menu (the command may open another window).
    /// Frees `self`: call it last.
    fn activate(&mut self, hwnd: HWND, target: Target) {
        let id = match target {
            Target::Control(i) => CONTROLS[i],
            Target::Item(i) => match self.entries[i] {
                Entry::Item { id, enabled: true, .. } => id,
                _ => return,
            },
        };
        let app = self.app.clone();
        self.close(hwnd);
        crate::tray_command(&app, id);
    }

    fn set_hover(&mut self, hwnd: HWND, hover: Option<Target>) {
        if !self.tracking {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            self.tracking = unsafe { TrackMouseEvent(&mut tme) }.is_ok();
        }
        if self.hover != hover {
            self.hover = hover;
            if hover.is_some() {
                self.focus = None;
            }
            invalidate(hwnd);
        }
    }
}

unsafe fn create(app: &AppHandle) -> Result<()> {
    static REGISTER: Once = Once::new();
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = w!("feathertune-tray-menu");
    REGISTER.call_once(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    });
    let state = app.state::<Player>().current();
    let font = |family: &str, size, weight, align| gfx::text_format(family, size, weight, align);
    let menu = Box::new(Menu {
        app: app.clone(),
        entries: entries(app),
        title: state.as_ref().map(|s| s.track.title.clone()).unwrap_or_default(),
        artist: state.as_ref().map(|s| s.track.artist.clone()).unwrap_or_default(),
        playing: state.as_ref().is_some_and(|s| !s.paused),
        has_track: state.is_some(),
        surface: gfx::Surface::default(),
        glows: gfx::GlowSets::default(),
        name: font(TEXT_FONT, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        small: font(TEXT_FONT, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        label: font(TEXT_FONT, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        icons: font(ICON_FONT, 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        hover: None,
        pressed: None,
        focus: None,
        tracking: false,
        captured: false,
        closing: false,
    });
    let total = height(&menu.entries);
    let mut cursor = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut cursor);
    }
    let menu = Box::into_raw(menu);
    // On success WM_NCCREATE takes the box (freed in WM_NCDESTROY). Made at the cursor first, so it
    // has the DPI of that monitor when it is sized.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!("feathertune menu"),
            WS_POPUP,
            cursor.x,
            cursor.y,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            Some(menu as *const _),
        )?
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    let scale = gfx::scale(hwnd);
    let (w, h) = ((WIDTH * scale).round() as i32, (total * scale).round() as i32);
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST), &mut info);
    }
    let work = info.rcWork;
    // Opens away from the edge the tray is on, inside the work area.
    let x = if cursor.x + w > work.right { cursor.x - w } else { cursor.x };
    let y = if cursor.y + h > work.bottom { cursor.y - h } else { cursor.y };
    let x = x.clamp(work.left, (work.right - w).max(work.left));
    let y = y.clamp(work.top, (work.bottom - h).max(work.top));
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            size_of_val(&corner) as u32,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        if !SetForegroundWindow(hwnd).as_bool() {
            SetCapture(hwnd);
            (*menu).captured = true;
        }
    }
    Ok(())
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let menu = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Menu;
        if menu.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let m = &mut *menu;
        let scale = gfx::scale(hwnd);
        let mouse = || ((lparam.0 as i16) as f32 / scale, ((lparam.0 >> 16) as i16) as f32 / scale);
        let outside = |(x, y): (f32, f32)| x < 0.0 || y < 0.0 || x >= WIDTH || y >= height(&m.entries);
        match msg {
            WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_INACTIVE => {
                m.close(hwnd);
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let hover = m.hit(mouse().0, mouse().1);
                m.set_hover(hwnd, hover);
            }
            WM_MOUSELEAVE => {
                m.tracking = false;
                if m.hover.take().is_some() | m.pressed.take().is_some() {
                    invalidate(hwnd);
                }
            }
            WM_LBUTTONDOWN => {
                if m.captured && outside(mouse()) {
                    m.close(hwnd);
                    return LRESULT(0);
                }
                m.pressed = m.hit(mouse().0, mouse().1);
                invalidate(hwnd);
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let (x, y) = mouse();
                let clicked = m.hit(x, y).filter(|t| m.pressed == Some(*t));
                m.pressed = None;
                invalidate(hwnd);
                // Last: running an item destroys the window and frees `m`.
                if let Some(t) = clicked {
                    m.activate(hwnd, t);
                }
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                if key == VK_ESCAPE.0 {
                    m.close(hwnd);
                } else if key == VK_DOWN.0 || key == VK_UP.0 {
                    m.focus = m.step_focus(if key == VK_DOWN.0 { 1 } else { -1 });
                    m.hover = None;
                    invalidate(hwnd);
                } else if (key == VK_RETURN.0 || key == VK_SPACE.0)
                    && let Some(i) = m.focus
                {
                    m.activate(hwnd, Target::Item(i));
                }
                return LRESULT(0);
            }
            WM_PAINT => {
                m.paint(hwnd);
                return LRESULT(0);
            }
            WM_DESTROY => {
                m.closing = true;
                if m.captured {
                    let _ = ReleaseCapture();
                }
                WINDOW.store(0, Ordering::Relaxed);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(menu));
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}
