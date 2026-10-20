//! Floating lyrics (plugin `desktop_lyrics`): the current line of the synced lyrics, and the next one
//! dimmer, over everything on the desktop. A native window drawn with Direct2D (gfx::Offscreen into a
//! layered window, so only the text and its outline are on screen); nothing here is HTML.
//!
//! - Opened and closed from the tray or the settings window; whether it is open and where it stands are
//!   kept in `desktop-lyrics.json` in the profile, so it comes back after a restart.
//! - Locked (the default) it never takes the mouse: clicks go to whatever is below. Unlocked (tray) it
//!   shows a panel to drag it by and a lock button.
//! - Fed by the `Player` state and the lines of `lyrics::Lyrics` (looked up when either this plugin or
//!   `synced_lyrics` is on). The time is `State::position_now()` with the page's 250 ms lead, so it
//!   changes lines with the site's Lyrics tab.
//! - Performance rules: it repaints only when the line or the sung word changes, on a one-shot timer
//!   set to the next change (no frame loop); a change cross-fades on a 60 fps timer that exists only
//!   for the fade. Hidden (no timers) while paused, between tracks or without synced lyrics. Layouts
//!   and the brush are kept until their line, word or the render target changes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Once};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_RANGE,
    DWRITE_WORD_WRAPPING_WRAP, IDWriteTextFormat, IDWriteTextLayout,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
    ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_EXSTYLE, GWLP_USERDATA, GetWindowLongPtrW,
    GetWindowRect, HTCAPTION, HTCLIENT, IDC_SIZEALL, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WM_APP, WM_DESTROY, WM_DPICHANGED, WM_EXITSIZEMOVE, WM_LBUTTONUP, WM_NCCREATE, WM_NCDESTROY,
    WM_NCHITTEST, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Result, w};

use crate::gfx::{self, ICON_FONT, point, rect, rgb, rgba};
use crate::lyrics::{Line, Lyrics, track_key};
use crate::player::{Player, State};
use crate::settings::Live;

/// Window size in DIPs: two lines of the current lyric and one of the next.
const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 128.0;
const PAD: f32 = 20.0;
const CURRENT_TOP: f32 = 8.0;
const CURRENT_BOTTOM: f32 = 88.0;
const NEXT_BOTTOM: f32 = 120.0;
/// The lock button of the unlocked panel.
const LOCK: (f32, f32, f32, f32) = (WIDTH - 40.0, 8.0, WIDTH - 8.0, 40.0);
/// The page lights a line this early (lyrics/page.js), so both change together.
const LEAD_MS: f64 = 250.0;
const FADE_MS: f32 = 320.0;
/// How far a line slides while it fades, in DIPs.
const SLIDE: f32 = 10.0;
const FRAME_MS: u32 = 16;
const LINE_TIMER: usize = 0xfeb0;
const FRAME_TIMER: usize = 0xfeb1;
const WM_UPDATE: u32 = WM_APP + 1;
const WM_LOCK: u32 = WM_APP + 2;
const FILE: &str = "desktop-lyrics.json";
/// The outline: the text drawn dark at these offsets under itself, then a soft drop below.
const OUTLINE: [(f32, f32); 8] =
    [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)];

/// The open window, 0 when closed. Only the main thread creates and destroys it.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static LOCKED: AtomicBool = AtomicBool::new(true);

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    open: bool,
    /// Top left in screen pixels.
    position: Option<(i32, i32)>,
}

fn saved_path(app: &AppHandle) -> PathBuf {
    app.state::<Live>().profile.join(FILE)
}

fn load(app: &AppHandle) -> Saved {
    std::fs::read(saved_path(app)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(app: &AppHandle, f: impl FnOnce(&mut Saved)) {
    let mut saved = load(app);
    f(&mut saved);
    if let Ok(bytes) = serde_json::to_vec(&saved)
        && let Err(e) = std::fs::write(saved_path(app), bytes)
    {
        eprintln!("floating lyrics: position not saved: {e}");
    }
}

/// Reopens it when it was open at the last exit. Setup.
pub fn start(app: &AppHandle) {
    if load(app).open {
        open(app);
    }
}

pub fn is_open() -> bool {
    WINDOW.load(Ordering::Relaxed) != 0
}

pub fn is_locked() -> bool {
    LOCKED.load(Ordering::Relaxed)
}

pub fn open(app: &AppHandle) {
    if !app.state::<Live>().plugin("desktop_lyrics") {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if is_open() {
            return;
        }
        match unsafe { create(&handle) } {
            Ok(()) => save(&handle, |s| s.open = true),
            Err(e) => eprintln!("floating lyrics unavailable: {e}"),
        }
    });
}

pub fn close(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let hwnd = WINDOW.load(Ordering::Relaxed);
        if hwnd != 0 {
            let _ = unsafe { DestroyWindow(HWND(hwnd as _)) };
            save(&handle, |s| s.open = false);
        }
    });
}

pub fn toggle(app: &AppHandle) {
    if is_open() { close(app) } else { open(app) }
}

/// Locked: clicks pass through. Unlocked: a panel to drag it by.
pub fn set_locked(locked: bool) {
    post(WM_LOCK, WPARAM(usize::from(locked)));
}

/// The lyrics or the player changed. Any thread.
pub fn refresh() {
    post(WM_UPDATE, WPARAM(0));
}

fn post(msg: u32, wparam: WPARAM) {
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), msg, wparam, LPARAM(0));
        }
    }
}

/// A lyric line on screen: its layout, and for a line timed word by word a second one with the sung
/// words lit (made with the target's brush, so it is rebuilt with the target).
struct Text {
    text: String,
    plain: IDWriteTextLayout,
    /// Timed word by word: it starts dim and its words light up.
    words: bool,
    /// UTF-16 length of the sung words.
    sung: u32,
    lit: Option<(u32, IDWriteTextLayout)>,
}

/// What the window shows: the track, the current line (-1: before the first) and its sung word.
struct Shown {
    key: String,
    index: isize,
    word: isize,
    current: Option<Text>,
    next: Option<IDWriteTextLayout>,
}

struct Floating {
    app: AppHandle,
    /// The worker stops when this is set.
    closed: Arc<AtomicBool>,
    bitmap: Option<gfx::Offscreen>,
    big: IDWriteTextFormat,
    small: IDWriteTextFormat,
    icons: IDWriteTextFormat,
    /// White, for the lit words; with the target generation it was made on.
    white: Option<(u32, ID2D1SolidColorBrush)>,
    shown: Option<Shown>,
    /// The content that is fading out, and since when.
    old: Option<(Shown, Instant)>,
    visible: bool,
    fading: bool,
}

unsafe fn create(app: &AppHandle) -> Result<()> {
    static REGISTER: Once = Once::new();
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = w!("feathertune-floating-lyrics");
    REGISTER.call_once(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_SIZEALL).unwrap_or_default(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    });
    let big = gfx::text_format(
        "Segoe UI Variable Display",
        28.0,
        DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_TEXT_ALIGNMENT_CENTER,
    )?;
    unsafe { big.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)? };
    let closed = Arc::new(AtomicBool::new(false));
    let floating = Box::new(Floating {
        app: app.clone(),
        closed: closed.clone(),
        bitmap: None,
        big,
        small: gfx::text_format(
            "Segoe UI Variable Text",
            17.0,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_TEXT_ALIGNMENT_CENTER,
        )?,
        icons: gfx::text_format(ICON_FONT, 14.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        white: None,
        shown: None,
        old: None,
        visible: false,
        fading: false,
    });
    let floating = Box::into_raw(floating);
    let mut style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    if is_locked() {
        style |= WS_EX_TRANSPARENT;
    }
    // On success WM_NCCREATE takes the box (freed in WM_NCDESTROY).
    let hwnd = unsafe {
        CreateWindowExW(
            style,
            class,
            w!("feathertune lyrics"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            Some(floating as *const _),
        )?
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    unsafe { place(app, hwnd) };
    let mut rx = app.state::<Player>().subscribe();
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        loop {
            match rx.blocking_recv() {
                Err(RecvError::Closed) => break,
                _ if closed.load(Ordering::Relaxed) => break,
                Ok(_) | Err(RecvError::Lagged(_)) => unsafe {
                    let _ = PostMessageW(Some(HWND(raw as _)), WM_UPDATE, WPARAM(0), LPARAM(0));
                },
            }
        }
    });
    refresh();
    Ok(())
}

/// Where it stood, or centered near the bottom of the primary monitor.
unsafe fn place(app: &AppHandle, hwnd: HWND) {
    let saved = load(app).position;
    let anchor = saved.map_or(POINT::default(), |(x, y)| POINT { x, y });
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(MonitorFromPoint(anchor, MONITOR_DEFAULTTOPRIMARY), &mut info) };
    // Sized for the DPI of the monitor it goes to: move first, then size.
    let (x, y) = saved.unwrap_or((info.rcWork.left, info.rcWork.top));
    let _ = unsafe { SetWindowPos(hwnd, None, x, y, 1, 1, SWP_NOZORDER | SWP_NOACTIVATE) };
    let scale = gfx::scale(hwnd);
    let (w, h) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
    let (x, y) = saved.unwrap_or_else(|| {
        let work = info.rcWork;
        ((work.left + work.right - w) / 2, work.bottom - h - (48.0 * scale) as i32)
    });
    let _ = unsafe { SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
}

/// The current line (-1 before the first) and, in a line timed by word, the sung word (-1: none yet).
fn position(lines: &[Line], ms: f64) -> (isize, isize) {
    let index = lines.partition_point(|l| f64::from(l.ms) <= ms) as isize - 1;
    let word = usize::try_from(index)
        .ok()
        .map_or(-1, |i| lines[i].words.partition_point(|(at, _)| f64::from(*at) <= ms) as isize - 1);
    (index, word)
}

/// When the line or the word changes next, in ms of track time.
fn next_change(lines: &[Line], index: isize, ms: f64) -> Option<f64> {
    let next_line = lines.get((index + 1) as usize).map(|l| f64::from(l.ms));
    let next_word = usize::try_from(index)
        .ok()
        .and_then(|i| lines[i].words.iter().map(|(at, _)| f64::from(*at)).find(|at| *at > ms));
    [next_line, next_word].into_iter().flatten().reduce(f64::min)
}

/// UTF-16 length of the words up to `word`, within the line's text.
fn sung_len(line: &Line, word: isize) -> u32 {
    let n: usize = line.words.iter().take((word + 1).max(0) as usize).map(|(_, t)| t.encode_utf16().count()).sum();
    n.min(line.text.encode_utf16().count()) as u32
}

impl Floating {
    /// What should be on screen now, from the player and the lyrics.
    fn wanted(&self) -> Option<(State, Arc<crate::lyrics::Synced>)> {
        let state = self.app.state::<Player>().current()?;
        let synced = self.app.state::<Lyrics>().current()?;
        (synced.key == track_key(&state.track)).then_some((state, synced))
    }

    fn text(&self, line: &Line, word: isize) -> Option<Text> {
        if line.text.is_empty() {
            return None;
        }
        let height = CURRENT_BOTTOM - CURRENT_TOP;
        let plain = gfx::text_layout(&line.text, &self.big, WIDTH - 2.0 * PAD, height).ok()?;
        let words = !line.words.is_empty();
        Some(Text {
            text: line.text.clone(),
            plain,
            words,
            sung: if words { sung_len(line, word) } else { 0 },
            lit: None,
        })
    }

    fn next_layout(&self, line: Option<&Line>) -> Option<IDWriteTextLayout> {
        let line = line.filter(|l| !l.text.is_empty())?;
        gfx::text_layout(&line.text, &self.small, WIDTH - 2.0 * PAD, NEXT_BOTTOM - CURRENT_BOTTOM).ok()
    }

    /// Brings the content up to date, shows or hides the window, draws it and sets the timers.
    fn update(&mut self, hwnd: HWND) {
        let wanted = self.wanted();
        let playing = wanted.as_ref().is_some_and(|(s, _)| !s.paused);
        let mut next_ms = None;
        if let Some((state, synced)) = wanted.as_ref().filter(|_| playing) {
            let lines = &synced.lines;
            let ms = state.position_now() * 1000.0 + LEAD_MS;
            let (index, word) = position(lines, ms);
            next_ms = next_change(lines, index, ms).map(|at| at - ms);
            let same_line = self.shown.as_ref().is_some_and(|s| s.key == synced.key && s.index == index);
            if !same_line {
                let line = usize::try_from(index).ok().map(|i| &lines[i]);
                let shown = Shown {
                    key: synced.key.clone(),
                    index,
                    word,
                    current: line.and_then(|l| self.text(l, word)),
                    next: self.next_layout(lines.get((index + 1) as usize)),
                };
                // Only a line change within the track cross-fades; a new track just appears.
                let old = self.shown.replace(shown).filter(|s| s.key == synced.key);
                self.old = old.map(|s| (s, Instant::now()));
            } else if let Some(shown) = self.shown.as_mut().filter(|s| s.word != word) {
                shown.word = word;
                if let (Some(text), Ok(i)) = (shown.current.as_mut(), usize::try_from(index)) {
                    text.sung = sung_len(&lines[i], word);
                    text.lit = None;
                }
            }
        } else {
            self.shown = None;
            self.old = None;
        }
        let has_text = self.shown.as_ref().is_some_and(|s| s.current.is_some() || s.next.is_some());
        let visible = !is_locked() || (playing && has_text);
        if visible != self.visible {
            self.visible = visible;
            unsafe {
                let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
        }
        if self.old.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() * 1000.0 >= FADE_MS) {
            self.old = None;
        }
        if visible {
            self.draw(hwnd);
        }
        unsafe {
            let _ = KillTimer(Some(hwnd), LINE_TIMER);
            if let Some(ms) = next_ms.filter(|_| visible) {
                SetTimer(Some(hwnd), LINE_TIMER, ms.clamp(10.0, 60_000.0) as u32, None);
            }
        }
        let fading = visible && self.old.is_some();
        if fading != self.fading {
            self.fading = fading;
            unsafe {
                if fading {
                    SetTimer(Some(hwnd), FRAME_TIMER, FRAME_MS, None);
                } else {
                    let _ = KillTimer(Some(hwnd), FRAME_TIMER);
                }
            }
        }
    }

    fn draw(&mut self, hwnd: HWND) {
        let scale = gfx::scale(hwnd);
        let (w, h) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
        if self.bitmap.as_ref().is_none_or(|b| b.width != w || b.height != h) {
            match gfx::Offscreen::new(w, h) {
                Ok(b) => self.bitmap = Some(b),
                Err(e) => {
                    eprintln!("floating lyrics: no bitmap: {e}");
                    return;
                }
            }
        }
        let t = self.old.as_ref().map_or(1.0, |(_, at)| (at.elapsed().as_secs_f32() * 1000.0 / FADE_MS).min(1.0));
        let t = 1.0 - (1.0 - t) * (1.0 - t);
        let locked = is_locked();
        let (big, small, icons) = (self.big.clone(), self.small.clone(), self.icons.clone());
        let white = &mut self.white;
        let (shown, old) = (self.shown.as_mut(), self.old.as_mut().map(|(s, _)| s));
        let Some(bitmap) = self.bitmap.as_mut() else { return };
        let result = bitmap.draw(scale, |c| {
            if !locked {
                c.pill(rect(0.0, 0.0, WIDTH, HEIGHT), 16.0, rgba(0x101018, 0.6));
                let (l, t, r, b) = LOCK;
                c.pill(rect(l, t, r, b), 16.0, rgba(0xffffff, 0.14));
                c.text("\u{e72e}", &icons, rect(l, t, r, b), rgb(0xffffff));
            }
            if white.as_ref().is_none_or(|(g, _)| *g != c.generation) {
                *white = c.solid(rgb(0xffffff)).map(|b| (c.generation, b));
            }
            let brush = white.as_ref().map(|(_, b)| b);
            if let Some(old) = old {
                paint(c, old, &big, brush, 1.0 - t, -SLIDE * t);
            }
            match shown {
                Some(s) => paint(c, s, &big, brush, t, SLIDE * (1.0 - t)),
                // Unlocked with nothing to show: say what it is, so it can be placed.
                None => c.text(
                    "Floating lyrics: drag to place, then lock",
                    &small,
                    rect(PAD, CURRENT_TOP, WIDTH - PAD, CURRENT_BOTTOM),
                    rgba(0xffffff, 0.8),
                ),
            }
        });
        if let Err(e) = result {
            eprintln!("floating lyrics: draw failed: {e}");
            return;
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let size = SIZE { cx: w, cy: h };
        unsafe {
            let _ = UpdateLayeredWindow(
                hwnd,
                None,
                None,
                Some(&size),
                Some(bitmap.dc()),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }
}

/// One content (current and next line) at `alpha`, moved down by `dy`.
fn paint(
    c: &gfx::Canvas,
    s: &mut Shown,
    big: &IDWriteTextFormat,
    white: Option<&ID2D1SolidColorBrush>,
    alpha: f32,
    dy: f32,
) {
    if alpha <= 0.0 {
        return;
    }
    if let Some(text) = s.current.as_mut() {
        let origin = point(PAD, CURRENT_TOP + dy);
        outlined(c, &text.plain, origin, alpha);
        if !text.words {
            c.layout(&text.plain, origin, rgba(0xffffff, alpha));
        } else {
            // Dim, then the sung words over it (the rest of the lit copy is drawn transparent).
            c.layout(&text.plain, origin, rgba(0xffffff, 0.5 * alpha));
            if let Some(white) = white.filter(|_| text.sung > 0) {
                if text.lit.as_ref().is_none_or(|(g, _)| *g != c.generation) {
                    text.lit = lit(text, big, white).map(|l| (c.generation, l));
                }
                if let Some((_, layout)) = &text.lit {
                    unsafe { white.SetOpacity(alpha) };
                    c.layout(layout, origin, rgba(0, 0.0));
                }
            }
        }
    }
    if let Some(next) = &s.next {
        let origin = point(PAD, CURRENT_BOTTOM + dy);
        outlined(c, next, origin, alpha * 0.8);
        c.layout(next, origin, rgba(0xffffff, 0.6 * alpha));
    }
}

/// The dark outline and drop under a layout, so it reads on any background.
fn outlined(c: &gfx::Canvas, layout: &IDWriteTextLayout, origin: windows_numerics::Vector2, alpha: f32) {
    for (dx, dy) in OUTLINE {
        c.layout(layout, point(origin.X + dx, origin.Y + dy), rgba(0x000000, 0.5 * alpha));
    }
    c.layout(layout, point(origin.X, origin.Y + 2.5), rgba(0x000000, 0.35 * alpha));
}

/// A copy of the line's layout with the sung words in `white`.
fn lit(text: &Text, big: &IDWriteTextFormat, white: &ID2D1SolidColorBrush) -> Option<IDWriteTextLayout> {
    let (width, height) = unsafe { (text.plain.GetMaxWidth(), text.plain.GetMaxHeight()) };
    let layout = gfx::text_layout(&text.text, big, width, height).ok()?;
    unsafe { layout.SetDrawingEffect(white, DWRITE_TEXT_RANGE { startPosition: 0, length: text.sung }).ok()? };
    Some(layout)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let floating = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Floating;
        if floating.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let f = &mut *floating;
        match msg {
            WM_NCHITTEST => {
                let mut p = POINT { x: lparam.0 as i16 as i32, y: (lparam.0 >> 16) as i16 as i32 };
                let _ = ScreenToClient(hwnd, &mut p);
                let scale = gfx::scale(hwnd);
                let (x, y) = (p.x as f32 / scale, p.y as f32 / scale);
                let (l, t, r, b) = LOCK;
                let on_lock = (l..r).contains(&x) && (t..b).contains(&y);
                return LRESULT(if on_lock { HTCLIENT } else { HTCAPTION } as isize);
            }
            WM_LBUTTONUP => {
                set_locked(true);
                return LRESULT(0);
            }
            WM_LOCK => {
                let locked = wparam.0 != 0;
                LOCKED.store(locked, Ordering::Relaxed);
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let transparent = WS_EX_TRANSPARENT.0 as isize;
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, if locked { style | transparent } else { style & !transparent });
                if locked {
                    save_position(&f.app, hwnd);
                }
                f.update(hwnd);
                return LRESULT(0);
            }
            WM_UPDATE => {
                f.update(hwnd);
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == LINE_TIMER || wparam.0 == FRAME_TIMER => {
                f.update(hwnd);
                return LRESULT(0);
            }
            WM_EXITSIZEMOVE => save_position(&f.app, hwnd),
            WM_DPICHANGED => {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                f.update(hwnd);
                return LRESULT(0);
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), LINE_TIMER);
                let _ = KillTimer(Some(hwnd), FRAME_TIMER);
                f.closed.store(true, Ordering::Relaxed);
                WINDOW.store(0, Ordering::Relaxed);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(floating));
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn save_position(app: &AppHandle, hwnd: HWND) {
    let mut r = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
        save(app, |s| s.position = Some((r.left, r.top)));
    }
}
