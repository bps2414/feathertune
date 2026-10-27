//! Floating lyrics (plugin `desktop_lyrics`): the current line of the synced lyrics, and the next one
//! dimmer, over everything on the desktop. A native window drawn with Direct2D (gfx::Offscreen into a
//! layered window, so only the text and its outline are on screen); nothing here is HTML.
//!
//! - Opened and closed from the tray or the settings window; whether it is open and where it stands are
//!   kept in `desktop-lyrics.json` in the profile, so it comes back after a restart.
//! - Locked (the default) it never takes the mouse: clicks go to whatever is below. Unlocked (tray) it
//!   shows a panel to drag it by and a lock button.
//! - The lines, their timing and their drawing are `lyric_view::View` (shared with the mini player).
//! - Performance rules: it repaints only when the line or the sung word changes, on a one-shot timer
//!   set to the next change (no frame loop); a change cross-fades on a 60 fps timer that exists only
//!   for the fade. Hidden (no timers) while paused, between tracks or without synced lyrics.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Once};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_WRAP,
    IDWriteTextFormat,
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

use crate::gfx::{self, ICON_FONT, rect, rgb, rgba};
use crate::i18n::t;
use crate::lyric_view::View;
use crate::player::Player;
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
const FRAME_MS: u32 = 16;
const LINE_TIMER: usize = 0xfeb0;
const FRAME_TIMER: usize = 0xfeb1;
const WM_UPDATE: u32 = WM_APP + 1;
const WM_LOCK: u32 = WM_APP + 2;
const FILE: &str = "desktop-lyrics.json";
/// The open window, 0 when closed. Only the main thread creates and destroys it.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static LOCKED: AtomicBool = AtomicBool::new(true);
/// Game mode closed it while it was open (`set_suspended`).
static SUSPENDED: AtomicBool = AtomicBool::new(false);

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

/// Game mode (game_mode.rs): closes the window without saving that, and opens it again afterwards if
/// it was open.
pub fn set_suspended(app: &AppHandle, on: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if on {
            let hwnd = WINDOW.load(Ordering::Relaxed);
            SUSPENDED.store(hwnd != 0, Ordering::Relaxed);
            if hwnd != 0 {
                let _ = unsafe { DestroyWindow(HWND(hwnd as _)) };
            }
        } else if SUSPENDED.swap(false, Ordering::Relaxed)
            && handle.state::<Live>().plugin("desktop_lyrics")
            && !is_open()
        {
            // Here, not through `open`: its own job would run after a quick second click had turned game
            // mode on again, and leave the window open during it.
            if let Err(e) = unsafe { create(&handle) } {
                eprintln!("floating lyrics unavailable: {e}");
            }
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

struct Floating {
    app: AppHandle,
    /// The worker stops when this is set.
    closed: Arc<AtomicBool>,
    bitmap: Option<gfx::Offscreen>,
    view: View,
    small: IDWriteTextFormat,
    icons: IDWriteTextFormat,
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
    let small =
        gfx::text_format("Segoe UI Variable Text", 17.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?;
    let closed = Arc::new(AtomicBool::new(false));
    let floating = Box::new(Floating {
        app: app.clone(),
        closed: closed.clone(),
        bitmap: None,
        view: View::new(
            big,
            small.clone(),
            WIDTH - 2.0 * PAD,
            CURRENT_BOTTOM - CURRENT_TOP,
            NEXT_BOTTOM - CURRENT_BOTTOM,
        ),
        small,
        icons: gfx::text_format(ICON_FONT, 14.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
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

impl Floating {
    /// Brings the content up to date, shows or hides the window, draws it when what it shows changed
    /// (`force`: the lock or the size did) and sets the timers.
    fn update(&mut self, hwnd: HWND, force: bool) {
        let update = self.view.update(&self.app, false);
        let visible = !is_locked() || (update.playing && self.view.has_text());
        let appeared = visible && !self.visible;
        if visible != self.visible {
            self.visible = visible;
            unsafe {
                let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
        }
        let fading = visible && self.view.fading();
        // A roll's frames, and the one after its end (at rest).
        if visible && (force || appeared || update.changed || fading || self.fading) {
            self.draw(hwnd);
        }
        unsafe {
            let _ = KillTimer(Some(hwnd), LINE_TIMER);
            if let Some(ms) = update.next_ms.filter(|_| visible) {
                SetTimer(Some(hwnd), LINE_TIMER, ms.clamp(10.0, 60_000.0) as u32, None);
            }
        }
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
        let locked = is_locked();
        let (small, icons, view) = (&self.small, &self.icons, &mut self.view);
        let Some(bitmap) = self.bitmap.as_mut() else { return };
        let result = bitmap.draw(scale, |c| {
            if !locked {
                c.pill(rect(0.0, 0.0, WIDTH, HEIGHT), 16.0, rgba(0x101018, 0.6));
                let (l, t, r, b) = LOCK;
                c.pill(rect(l, t, r, b), 16.0, rgba(0xffffff, 0.14));
                c.text("\u{e72e}", icons, rect(l, t, r, b), rgb(0xffffff));
            }
            if view.is_empty() {
                // Unlocked with nothing to show: say what it is, so it can be placed.
                let r = rect(PAD, CURRENT_TOP, WIDTH - PAD, CURRENT_BOTTOM);
                c.text(t("Floating lyrics: drag to place, then lock"), small, r, rgba(0xffffff, 0.8));
            } else {
                view.draw(c, PAD, CURRENT_TOP, rgb(0xffffff), true);
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
                f.update(hwnd, true);
                return LRESULT(0);
            }
            WM_UPDATE => {
                f.update(hwnd, false);
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == LINE_TIMER || wparam.0 == FRAME_TIMER => {
                f.update(hwnd, false);
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
                f.update(hwnd, true);
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
