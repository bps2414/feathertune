//! Launcher: a small start screen (Direct2D, like the mini player) that explains what the app is
//! doing while it starts: checking for updates (updater.rs), downloading and installing one, then
//! waiting for the player page. The main window is created hidden and shown when both the update
//! check and the page load are done (or after `TIMEOUT`), so it never opens on a blank page.
//!
//! - Only an interactive start uses it (not `--hidden`, `--mini-player` or `--measure`), and only with
//!   the `launcher` plugin on.
//! - While the progress is unknown a short bar sweeps on one timer, which runs only while the screen
//!   exists; a download shows its real progress.
//! - It lives on the main thread; `set` and `step_done` can be called from any thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetClientRect, GetCursorPos, GetWindowLongPtrW, HTCAPTION, IDC_ARROW, KillTimer, LoadCursorW, PostMessageW,
    RegisterClassExW, SW_SHOW, SWP_NOZORDER, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, WM_APP, WM_CLOSE, WM_DESTROY, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSEXW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{Result, w};

use crate::gfx::{self, TEXT_FONT, rect};
use crate::updater;

/// Steps `step_done` waits for.
pub const UPDATE: u8 = 1;
pub const PAGE: u8 = 2;

/// The screen stays at most this long: a page that never finishes loading must not trap the user.
const TIMEOUT: Duration = Duration::from_secs(20);
/// How long after the page's `load` the site may take to draw its first content.
const PAINT_WAIT: Duration = Duration::from_secs(8);
const CONTENT_PROBE: &str = "(() => { const q = (s) => document.querySelector(s); return !!q('ytmusic-app-layout') && \
    !!q('ytmusic-shelf-renderer, ytmusic-carousel-shelf-renderer, ytmusic-player-bar .title, \
    ytmusic-two-column-browse-results-renderer') })()";
/// Window size in DIPs, and the progress bar inside it.
const WIDTH: f32 = 380.0;
const HEIGHT: f32 = 220.0;
const BAR_LEFT: f32 = 70.0;
const BAR_RIGHT: f32 = WIDTH - 70.0;
const BAR_TOP: f32 = 156.0;
const BAR_HEIGHT: f32 = 4.0;
/// The sweeping piece and the time it takes to cross the bar.
const SWEEP_WIDTH: f32 = 90.0;
const SWEEP_MS: u128 = 1400;
const NAME_FONT: &str = "Segoe UI Variable Display";
const WM_UPDATE: u32 = WM_APP + 4;
const TIMER: usize = 0xfeaa;
/// 30 fps, like the title bar.
const FRAME_MS: u32 = 33;

/// The open window, 0 when closed.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The screen is up and has not been dismissed.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The installer is running: the app is about to be replaced, so the timeout must not show the window.
static INSTALLING: AtomicBool = AtomicBool::new(false);
/// Steps still to be done.
static PENDING: AtomicU8 = AtomicU8::new(UPDATE | PAGE);

struct View {
    status: String,
    /// `None`: unknown, the bar sweeps.
    fraction: Option<f32>,
}

static VIEW: Mutex<View> = Mutex::new(View { status: String::new(), fraction: None });

struct Splash {
    surface: gfx::Surface,
    name: IDWriteTextFormat,
    status: IDWriteTextFormat,
    small: IDWriteTextFormat,
    start: Instant,
    ticking: bool,
}

/// Shows the screen at once (painted before the call returns, since the event loop is not running
/// yet) and starts the update check. Main thread. `false`: no screen, show the window as usual.
pub fn show(app: &AppHandle, profile: PathBuf) -> bool {
    // Set before the window exists, so the first paint has the text.
    set("Checking for updates…", None);
    match unsafe { create() } {
        Ok(()) => {
            ACTIVE.store(true, Ordering::Relaxed);
            let app = app.clone();
            std::thread::spawn({
                let app = app.clone();
                move || {
                    std::thread::sleep(TIMEOUT);
                    if !INSTALLING.load(Ordering::Relaxed) {
                        finish(&app);
                    }
                }
            });
            std::thread::spawn(move || run(&app, &profile));
            true
        }
        Err(e) => {
            eprintln!("start screen unavailable: {e}");
            false
        }
    }
}

/// Update check, download and install; then the update step is done.
fn run(app: &AppHandle, profile: &std::path::Path) {
    if updater::installed(profile) {
        match updater::check() {
            Ok(Some(update)) => {
                let text = |fraction: Option<f32>| match fraction {
                    Some(f) => format!("Downloading version {} ({:.0}%)", update.version, f * 100.0),
                    None => format!("Downloading version {}…", update.version),
                };
                set(&text(Some(0.0)), Some(0.0));
                match updater::install(&update, profile, |f| set(&text(f), f)) {
                    Ok(()) => {
                        INSTALLING.store(true, Ordering::Relaxed);
                        set(&format!("Installing version {}, feathertune will restart", update.version), None);
                        // The installer closes this process; leave before it has to.
                        std::thread::sleep(Duration::from_millis(1200));
                        app.exit(0);
                        return;
                    }
                    Err(e) => {
                        eprintln!("update failed: {e}");
                        set("The update failed, starting the current version", None);
                        std::thread::sleep(Duration::from_millis(1500));
                    }
                }
            }
            Ok(None) => {}
            // Offline is the usual reason: not worth the user's attention.
            Err(e) => eprintln!("update check failed: {e}"),
        }
    }
    set("Starting the player…", None);
    step_done(app, UPDATE);
}

/// Updates the text and the progress (`None`: unknown). Any thread.
pub fn set(status: &str, fraction: Option<f32>) {
    {
        let mut v = VIEW.lock().unwrap();
        v.status = status.into();
        v.fraction = fraction;
    }
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_UPDATE, WPARAM(0), LPARAM(0));
        }
    }
}

/// The page's `load` event: the site is a single-page app that draws itself afterwards, so the
/// screen stays until its first content is on the page (or `PAINT_WAIT` passes: a sign-in or error
/// page never has it) and a moment longer for the paint. Called on every load; only the first counts.
pub fn page_loaded(app: &AppHandle) {
    static WATCHING: AtomicBool = AtomicBool::new(false);
    if !ACTIVE.load(Ordering::Relaxed) || WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    if PENDING.load(Ordering::Relaxed) & UPDATE == 0 {
        set("Loading your music…", None);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let (start, ready) = (Instant::now(), Arc::new(AtomicBool::new(false)));
        while ACTIVE.load(Ordering::Relaxed) && start.elapsed() < PAINT_WAIT && !ready.load(Ordering::Relaxed) {
            if let Some(site) = app.get_webview("site") {
                let ready = ready.clone();
                crate::eval_with(&site, CONTENT_PROBE, move |r| {
                    if r == "true" {
                        ready.store(true, Ordering::Relaxed);
                    }
                });
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(300));
        step_done(&app, PAGE);
    });
}

/// Marks a step done; the last one dismisses the screen and shows the window. Any thread; does
/// nothing when there is no screen.
pub fn step_done(app: &AppHandle, step: u8) {
    if ACTIVE.load(Ordering::Relaxed) && PENDING.fetch_and(!step, Ordering::Relaxed) & !step == 0 {
        finish(app);
    }
}

fn finish(app: &AppHandle) {
    if !ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let hwnd = WINDOW.load(Ordering::Relaxed);
        if hwnd != 0 {
            let _ = unsafe { DestroyWindow(HWND(hwnd as _)) };
        }
        crate::show_main(&handle);
    });
}

unsafe fn create() -> Result<()> {
    static REGISTER: Once = Once::new();
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = w!("feathertune-start-screen");
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
    let center = |family, size, weight| gfx::text_format(family, size, weight, DWRITE_TEXT_ALIGNMENT_CENTER);
    let splash = Box::new(Splash {
        surface: gfx::Surface::default(),
        name: center(NAME_FONT, 30.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
        status: center(TEXT_FONT, 13.0, DWRITE_FONT_WEIGHT_NORMAL)?,
        small: center(TEXT_FONT, 11.0, DWRITE_FONT_WEIGHT_NORMAL)?,
        start: Instant::now(),
        ticking: false,
    });
    let splash = Box::into_raw(splash);
    // On success WM_NCCREATE takes the box (freed in WM_NCDESTROY).
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("feathertune"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            Some(splash as *const _),
        )?
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    unsafe {
        // Centered on the monitor the cursor is on, where the user just launched it.
        let scale = gfx::scale(hwnd);
        let (w, h) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
        let mut cursor = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST), &mut info);
        let area = info.rcWork;
        let (x, y) = (area.left + (area.right - area.left - w) / 2, area.top + (area.bottom - area.top - h) / 2);
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            size_of_val(&corner) as u32,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        // The event loop is not running yet: paint now, not when it gets to WM_PAINT.
        let _ = UpdateWindow(hwnd);
        (*splash).schedule(hwnd);
    }
    Ok(())
}

impl Splash {
    /// The sweep runs only while the progress is unknown.
    fn schedule(&mut self, hwnd: HWND) {
        let run = VIEW.lock().unwrap().fraction.is_none();
        if run == self.ticking {
            return;
        }
        self.ticking = run;
        unsafe {
            if run {
                SetTimer(Some(hwnd), TIMER, FRAME_MS, None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER);
            }
        }
    }

    fn paint(&mut self, hwnd: HWND) {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        let pal = gfx::palette();
        let (status, fraction) = {
            let v = VIEW.lock().unwrap();
            (v.status.clone(), v.fraction)
        };
        let sweep = self.ticking.then(|| (self.start.elapsed().as_millis() % SWEEP_MS) as f32 / SWEEP_MS as f32);
        let (name, status_format, small) = (&self.name, &self.status, &self.small);
        self.surface.paint(hwnd, client, |c| {
            c.fill(rect(0.0, 0.0, WIDTH, HEIGHT), pal.background);
            c.text("feathertune", name, rect(0.0, 56.0, WIDTH, 100.0), pal.text);
            c.text(&status, status_format, rect(16.0, 112.0, WIDTH - 16.0, 140.0), gfx::with_alpha(pal.text, 0.75));
            let track = rect(BAR_LEFT, BAR_TOP, BAR_RIGHT, BAR_TOP + BAR_HEIGHT);
            c.pill(track, BAR_HEIGHT / 2.0, gfx::with_alpha(pal.text, 0.16));
            let (from, to) = match (fraction, sweep) {
                (Some(f), _) => (BAR_LEFT, BAR_LEFT + (BAR_RIGHT - BAR_LEFT) * f.clamp(0.0, 1.0)),
                (None, Some(t)) => {
                    let t = t * t * (3.0 - 2.0 * t);
                    let left = BAR_LEFT - SWEEP_WIDTH + (BAR_RIGHT - BAR_LEFT + SWEEP_WIDTH) * t;
                    (left.max(BAR_LEFT), (left + SWEEP_WIDTH).min(BAR_RIGHT))
                }
                (None, None) => {
                    let mid = (BAR_LEFT + BAR_RIGHT) / 2.0;
                    (mid - SWEEP_WIDTH / 2.0, mid + SWEEP_WIDTH / 2.0)
                }
            };
            if to - from > BAR_HEIGHT {
                c.pill(rect(from, BAR_TOP, to, BAR_TOP + BAR_HEIGHT), BAR_HEIGHT / 2.0, pal.text);
            }
            c.text(
                concat!("v", env!("CARGO_PKG_VERSION")),
                small,
                rect(0.0, HEIGHT - 32.0, WIDTH, HEIGHT - 12.0),
                gfx::with_alpha(pal.text, 0.45),
            );
        });
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let splash = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Splash;
        if splash.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let s = &mut *splash;
        match msg {
            // The whole window drags it.
            WM_NCHITTEST => return LRESULT(HTCAPTION as isize),
            // Alt+F4 does nothing: the screen goes away when the start is done.
            WM_CLOSE => return LRESULT(0),
            WM_PAINT => {
                s.paint(hwnd);
                return LRESULT(0);
            }
            WM_UPDATE => {
                s.schedule(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            WM_TIMER if wparam.0 == TIMER => {
                let scale = gfx::scale(hwnd);
                let bar = RECT {
                    left: (BAR_LEFT * scale) as i32,
                    top: ((BAR_TOP - 2.0) * scale) as i32,
                    right: (BAR_RIGHT * scale).ceil() as i32,
                    bottom: ((BAR_TOP + BAR_HEIGHT + 2.0) * scale).ceil() as i32,
                };
                let _ = InvalidateRect(Some(hwnd), Some(&bar), false);
                return LRESULT(0);
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER);
                WINDOW.store(0, Ordering::Relaxed);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(splash));
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}
