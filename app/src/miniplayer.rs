//! Mini player: a small native window, always on top, with the artwork, title, artist, progress and
//! previous / play-pause / next, plus "back to the main window" and close. Drawn with Direct2D
//! (gfx.rs); nothing here is HTML.
//!
//! - Fed only by the `Player` state; the buttons run the same `controls` commands as the tray.
//! - Opening it hides the main window (memory Low, as in the tray); "back" shows the main window and
//!   closes it; close leaves the app in the tray. It exists only while open.
//! - The artwork URL comes from the validated state (https, one of the site's image hosts). A worker
//!   thread loads it through artwork.rs (download and WIC decode into a small premultiplied BGRA
//!   buffer); the window turns that into a Direct2D bitmap when it paints.
//! - With the `album_color` plugin the background is the artwork's ink with a few static glows of its
//!   colors (gfx::Theme); a change of theme flows in over `gfx::fading()`, on a timer that exists only
//!   during the flow. Without a theme the background stays flat.
//! - The worker also wakes once per second while playing to move the progress bar.
//! - The lyrics button grows it by a band with the current line of the synced lyrics and the next one
//!   (`lyric_view::View`, like the floating lyrics, in the window's colors). Whether it is open is
//!   kept in `mini-player.json` in the profile. A one-shot timer is set to the next line or word and a
//!   60 fps frame timer runs only for the short roll to the next line; while paused the line stays,
//!   with no timer.
//! - The top buttons (lyrics, full window, close) show what they do in a small label on hover.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_PROPERTIES, D2D1_ROUNDED_RECT, ID2D1BitmapBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_WRAP,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetClientRect, GetWindowLongPtrW, GetWindowRect, HTCAPTION, HTCLIENT, IDC_ARROW, KillTimer, LoadCursorW,
    PostMessageW, RegisterClassExW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetTimer, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WM_APP, WM_DESTROY, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_NCMOUSEMOVE, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Result, w};
use windows_numerics::Matrix3x2;

use crate::artwork::{self, Art};
use crate::gfx::{self, ICON_FONT, TEXT_FONT, point, rect};
use crate::lyric_view::View;
use crate::player::{Event, Player, State};
use crate::settings::Live;

/// Window size in DIPs, without and with the lyrics band.
const WIDTH: f32 = 340.0;
const HEIGHT: f32 = 104.0;
const LYRICS_BAND: f32 = 84.0;
/// The lyrics band: current line (two lines of text) and the next one.
const LYRIC_TOP: f32 = HEIGHT + 4.0;
const LYRIC_CURRENT: f32 = 50.0;
const LYRIC_NEXT: f32 = 24.0;
const ART: f32 = 88.0;
const ART_RADIUS: f32 = 8.0;
/// The static glows of a theme: center, radii (DIPs) and strength, kept soft so the white text stays
/// readable (the glows' luminance is capped in album_color.rs).
const GLOWS: [(f32, f32, f32, f32, f32); 3] =
    [(290.0, 0.0, 190.0, 110.0, 0.55), (130.0, HEIGHT, 170.0, 70.0, 0.45), (WIDTH, HEIGHT, 140.0, 60.0, 0.4)];
const WM_UPDATE: u32 = WM_APP + 1;
const TIMER: usize = 0xfea9;
/// One-shot, at the next lyric line or word.
const LYRIC_TIMER: usize = 0xfeb2;
const FILE: &str = "mini-player.json";
/// 30 fps, like the title bar; only while a theme flow runs.
const FRAME_MS: u32 = 33;
/// 60 fps while a lyric line rolls (under half a second), so the motion is smooth.
const LYRIC_FRAME_MS: u32 = 16;
use crate::titlebar::WM_MOUSELEAVE;

/// The open window, 0 when closed. Only the main thread creates and destroys it.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// Where it was last closed, to reopen there.
static LAST_POSITION: Mutex<Option<(i32, i32)>> = Mutex::new(None);
/// The lyrics band is open (saved in `FILE`).
static LYRICS: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Saved {
    lyrics: bool,
}

fn saved_path(app: &AppHandle) -> std::path::PathBuf {
    app.state::<Live>().profile.join(FILE)
}

/// The window height in DIPs.
fn height() -> f32 {
    if LYRICS.load(Ordering::Relaxed) { HEIGHT + LYRICS_BAND } else { HEIGHT }
}

pub fn is_open() -> bool {
    WINDOW.load(Ordering::Relaxed) != 0
}

/// Opens the mini player and hides the main window. Does nothing when the plugin is off.
pub fn open(app: &AppHandle) {
    if !app.state::<Live>().plugin("mini_player") {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !is_open()
            && let Err(e) = unsafe { create(&handle) }
        {
            eprintln!("mini player unavailable: {e}");
            return;
        }
        crate::set_hidden(&handle, true);
    });
}

pub fn close(app: &AppHandle) {
    let _ = app.run_on_main_thread(|| {
        let hwnd = WINDOW.load(Ordering::Relaxed);
        if hwnd != 0 {
            let _ = unsafe { DestroyWindow(HWND(hwnd as _)) };
        }
    });
}

/// Open: back to the main window. Closed: open it.
pub fn toggle(app: &AppHandle) {
    if is_open() { crate::show_main(app) } else { open(app) }
}

/// Repaints it and starts the flow to a new theme (after `gfx::set_theme`). Any thread.
pub fn refresh() {
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_UPDATE, WPARAM(0), LPARAM(0));
        }
    }
}

/// What the worker thread hands to the window.
#[derive(Default)]
struct Data {
    state: Option<State>,
    art: Option<Arc<Art>>,
    closed: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Previous,
    PlayPause,
    Next,
    Lyrics,
    Back,
    Close,
}

const BUTTONS: [Button; 6] =
    [Button::Previous, Button::PlayPause, Button::Next, Button::Lyrics, Button::Back, Button::Close];

struct Mini {
    app: AppHandle,
    data: Arc<Mutex<Data>>,
    surface: gfx::Surface,
    title: IDWriteTextFormat,
    artist: IDWriteTextFormat,
    icons: IDWriteTextFormat,
    small_icons: IDWriteTextFormat,
    /// The label under a hovered top button.
    hint: IDWriteTextFormat,
    lyrics: View,
    /// The lyrics' one-shot timer is set.
    lyric_timer: bool,
    /// The artwork as a bitmap brush of the current target (so it can fill a rounded square): URL and
    /// target generation it was made for.
    bitmap: Option<(String, u32, ID2D1BitmapBrush)>,
    /// The glow brushes of the themes on screen.
    glows: gfx::GlowSets,
    /// The frame timer's interval, when it runs (a theme flow or a lyric roll).
    ticking: Option<u32>,
    hover: Option<Button>,
    pressed: Option<Button>,
    tracking: bool,
}

fn button_rect(b: Button) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    let x = 8.0 + ART + 8.0;
    match b {
        Button::Previous => rect(x, 55.0, x + 40.0, 87.0),
        Button::PlayPause => rect(x + 44.0, 52.0, x + 82.0, 90.0),
        Button::Next => rect(x + 86.0, 55.0, x + 126.0, 87.0),
        Button::Lyrics => rect(WIDTH - 100.0, 6.0, WIDTH - 70.0, 36.0),
        Button::Back => rect(WIDTH - 68.0, 6.0, WIDTH - 38.0, 36.0),
        Button::Close => rect(WIDTH - 36.0, 6.0, WIDTH - 6.0, 36.0),
    }
}

/// What a top button does, shown under it on hover.
fn hint_text(b: Button, lyrics_open: bool) -> Option<&'static str> {
    match b {
        Button::Lyrics if lyrics_open => Some("Hide lyrics"),
        Button::Lyrics => Some("Show lyrics"),
        Button::Back => Some("Open the full window"),
        Button::Close => Some("Close mini player"),
        _ => None,
    }
}

fn hit(x: f32, y: f32) -> Option<Button> {
    BUTTONS.into_iter().find(|b| {
        let r = button_rect(*b);
        (r.left..r.right).contains(&x) && (r.top..r.bottom).contains(&y)
    })
}

unsafe fn create(app: &AppHandle) -> Result<()> {
    static REGISTER: Once = Once::new();
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = w!("feathertune-mini-player");
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
    let data = Arc::new(Mutex::new(Data { state: app.state::<Player>().current(), ..Default::default() }));
    let font = |family: &str, size, weight, align| gfx::text_format(family, size, weight, align);
    let saved: Saved =
        std::fs::read(saved_path(app)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    LYRICS.store(saved.lyrics, Ordering::Relaxed);
    let current = font("Segoe UI Variable Display", 16.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER)?;
    unsafe {
        current.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        // A one-line current line sits in the middle of its two-line box, not glued to the top.
        current.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    }
    let next = font("Segoe UI Variable Text", 12.5, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?;
    let mini = Box::new(Mini {
        app: app.clone(),
        data: data.clone(),
        surface: gfx::Surface::default(),
        title: font(TEXT_FONT, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        artist: font(TEXT_FONT, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        icons: font(ICON_FONT, 16.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        small_icons: font(ICON_FONT, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        hint: font(TEXT_FONT, 11.5, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        lyrics: View::new(current, next, WIDTH - 24.0, LYRIC_CURRENT, LYRIC_NEXT),
        lyric_timer: false,
        bitmap: None,
        glows: gfx::GlowSets::default(),
        ticking: None,
        hover: None,
        pressed: None,
        tracking: false,
    });
    let mini = Box::into_raw(mini);
    // On success WM_NCCREATE takes the box (freed in WM_NCDESTROY).
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
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
            Some(mini as *const _),
        )?
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    unsafe {
        place(app, hwnd);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            size_of_val(&corner) as u32,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    let rx = app.state::<Player>().subscribe();
    let hwnd = hwnd.0 as isize;
    std::thread::spawn(move || worker(hwnd, data, rx));
    // The lyrics band, if open, without waiting for a player event.
    refresh();
    Ok(())
}

/// Where it was last closed, or the bottom right of the main window's monitor.
unsafe fn place(app: &AppHandle, hwnd: HWND) {
    let scale = gfx::scale(hwnd);
    let (w, h) = ((WIDTH * scale).round() as i32, (height() * scale).round() as i32);
    let (x, y) = match *LAST_POSITION.lock().unwrap() {
        Some(p) => p,
        None => {
            let main = app.get_window("main").and_then(|w| w.hwnd().ok()).map(|h| HWND(h.0)).unwrap_or(hwnd);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = unsafe { GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTOPRIMARY), &mut info) };
            let margin = (16.0 * scale) as i32;
            (info.rcWork.right - w - margin, info.rcWork.bottom - h - margin)
        }
    };
    let _ = unsafe { SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
}

fn worker(hwnd: isize, data: Arc<Mutex<Data>>, mut rx: Receiver<Event>) {
    let post = || unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as _)), WM_UPDATE, WPARAM(0), LPARAM(0));
    };
    let mut fetched: Option<String> = None;
    loop {
        let (wanted, playing) = {
            let d = data.lock().unwrap();
            if d.closed {
                break;
            }
            let s = d.state.as_ref();
            (s.and_then(|s| s.track.artwork.clone()), s.is_some_and(|s| !s.paused))
        };
        if wanted != fetched {
            fetched = wanted.clone();
            let art = wanted.and_then(|url| match artwork::load(&url) {
                Ok(a) => Some(a),
                Err(e) => {
                    eprintln!("mini player: artwork not loaded: {e}");
                    None
                }
            });
            data.lock().unwrap().art = art;
            post();
        }
        // The timer is created inside the future: it needs the runtime's context.
        match tauri::async_runtime::block_on(async { tokio::time::timeout(Duration::from_secs(1), rx.recv()).await }) {
            Ok(Ok(event)) => {
                data.lock().unwrap().state = Some(event.state().clone());
                post();
            }
            Ok(Err(RecvError::Lagged(_))) => {}
            Ok(Err(RecvError::Closed)) => break,
            // The progress bar moves while playing.
            Err(_) if playing => post(),
            Err(_) => {}
        }
    }
}

impl Mini {
    fn set_hover(&mut self, hwnd: HWND, hover: Option<Button>) {
        if hover.is_some() && !self.tracking {
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
            invalidate(hwnd);
        }
    }

    /// Runs the frame timer only while the colors flow to a new theme or a lyric line rolls.
    fn schedule(&mut self, hwnd: HWND) {
        let lyrics_rolling = LYRICS.load(Ordering::Relaxed) && self.lyrics.fading();
        let run = if lyrics_rolling {
            Some(LYRIC_FRAME_MS)
        } else if gfx::fading() {
            Some(FRAME_MS)
        } else {
            None
        };
        if run == self.ticking {
            return;
        }
        self.ticking = run;
        unsafe {
            match run {
                Some(ms) => {
                    SetTimer(Some(hwnd), TIMER, ms, None);
                }
                None => {
                    let _ = KillTimer(Some(hwnd), TIMER);
                }
            }
        }
    }

    /// Brings the lyrics band up to date and sets its one-shot timer.
    fn update_lyrics(&mut self, hwnd: HWND) {
        let next = if LYRICS.load(Ordering::Relaxed) { self.lyrics.update(&self.app, true).next_ms } else { None };
        unsafe {
            if self.lyric_timer {
                let _ = KillTimer(Some(hwnd), LYRIC_TIMER);
            }
            self.lyric_timer = next.is_some();
            if let Some(ms) = next {
                SetTimer(Some(hwnd), LYRIC_TIMER, ms.clamp(10.0, 60_000.0) as u32, None);
            }
        }
        self.schedule(hwnd);
    }

    /// Opens or closes the lyrics band; the window grows or shrinks upward (its bottom stays).
    fn toggle_lyrics(&mut self, hwnd: HWND) {
        let on = !LYRICS.load(Ordering::Relaxed);
        LYRICS.store(on, Ordering::Relaxed);
        if let Ok(bytes) = serde_json::to_vec(&Saved { lyrics: on })
            && let Err(e) = std::fs::write(saved_path(&self.app), bytes)
        {
            eprintln!("mini player: not saved: {e}");
        }
        let scale = gfx::scale(hwnd);
        let mut r = RECT::default();
        unsafe {
            if GetWindowRect(hwnd, &mut r).is_ok() {
                let h = (height() * scale).round() as i32;
                let _ =
                    SetWindowPos(hwnd, None, r.left, r.bottom - h, r.right - r.left, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        self.update_lyrics(hwnd);
        invalidate(hwnd);
    }

    fn paint(&mut self, hwnd: HWND) {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        let pal = gfx::palette();
        let themes = gfx::blend();
        // Both themes of a flow are drawn, each at its share of the opacity.
        let layers: Vec<(gfx::Theme, f32)> = [(themes.from, 1.0 - themes.t), (themes.to, themes.t)]
            .into_iter()
            .filter_map(|(theme, share)| theme.map(|t| (t, share)))
            .collect();
        let sets = &mut self.glows;
        let data = self.data.clone();
        let data = data.lock().unwrap();
        let state = data.state.as_ref();
        let generation = self.surface.generation;
        let (hover, pressed) = (self.hover, self.pressed);
        let bitmap = &mut self.bitmap;
        let (title, artist, icons, small) = (&self.title, &self.artist, &self.icons, &self.small_icons);
        let hint = &self.hint;
        let lyrics_open = LYRICS.load(Ordering::Relaxed);
        let full_height = height();
        let lyrics = &mut self.lyrics;
        let art = data.art.as_ref().filter(|a| Some(&a.url) == state.and_then(|s| s.track.artwork.as_ref()));
        self.surface.paint(hwnd, client, |c| {
            c.fill(rect(0.0, 0.0, WIDTH, full_height), pal.background);
            let themes: Vec<gfx::Theme> = layers.iter().map(|(t, _)| *t).collect();
            sets.prepare(c, &themes);
            for (theme, share) in &layers {
                let Some(set) = sets.get(theme) else { continue };
                for (brush, (x, y, rx, ry, alpha)) in set.iter().zip(GLOWS) {
                    unsafe {
                        brush.SetCenter(point(x, y));
                        brush.SetRadiusX(rx);
                        brush.SetRadiusY(ry);
                        brush.SetOpacity(alpha * share);
                    }
                    c.fill_with(rect(0.0, 0.0, WIDTH, full_height), brush);
                }
            }
            let art_rect = rect(8.0, 8.0, 8.0 + ART, 8.0 + ART);
            // A soft shadow under the artwork.
            for (grow, alpha) in [(6.0, 0.05), (3.0, 0.08), (1.0, 0.12)] {
                let r = rect(
                    art_rect.left - grow,
                    art_rect.top - grow + 2.0,
                    art_rect.right + grow,
                    art_rect.bottom + grow + 2.0,
                );
                c.pill(r, ART_RADIUS + grow, gfx::rgba(0, alpha));
            }
            // The surface may have been rebuilt since the bitmap was made (device lost).
            let fresh = |b: &(String, u32, ID2D1BitmapBrush), a: &Art| b.0 == a.url && b.1 == generation;
            match art {
                Some(a) if bitmap.as_ref().is_none_or(|b| !fresh(b, a)) => {
                    let props = D2D1_BITMAP_PROPERTIES {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                    };
                    let size = D2D_SIZE_U { width: a.width, height: a.height };
                    *bitmap = unsafe {
                        c.target
                            .CreateBitmap(size, Some(a.pixels.as_ptr() as _), a.width * 4, &props)
                            .and_then(|b| c.target.CreateBitmapBrush(&b, None, None))
                    }
                    .ok()
                    .map(|brush| {
                        // Maps the center square of the image (video thumbnails are 16:9) onto the art.
                        let side = a.width.min(a.height) as f32;
                        let (x, y) = ((a.width as f32 - side) / 2.0, (a.height as f32 - side) / 2.0);
                        let k = ART / side;
                        let m = Matrix3x2 {
                            M11: k,
                            M12: 0.0,
                            M21: 0.0,
                            M22: k,
                            M31: art_rect.left - x * k,
                            M32: art_rect.top - y * k,
                        };
                        unsafe { brush.SetTransform(&m) };
                        (a.url.clone(), generation, brush)
                    });
                }
                Some(_) => {}
                None => *bitmap = None,
            }
            match (art, bitmap.as_ref()) {
                (Some(_), Some((_, _, b))) => unsafe {
                    let rounded = D2D1_ROUNDED_RECT { rect: art_rect, radiusX: ART_RADIUS, radiusY: ART_RADIUS };
                    c.target.FillRoundedRectangle(&rounded, b);
                },
                _ => {
                    c.pill(art_rect, ART_RADIUS, pal.hover);
                    c.text("\u{e8d6}", icons, art_rect, gfx::with_alpha(pal.text, 0.6));
                }
            }

            let x = 8.0 + ART + 8.0;
            let text_right = button_rect(Button::Lyrics).left - 4.0;
            match state {
                Some(s) => {
                    c.text(&s.track.title, title, rect(x, 10.0, text_right, 30.0), pal.text);
                    c.text(&s.track.artist, artist, rect(x, 30.0, text_right, 50.0), gfx::with_alpha(pal.text, 0.7));
                }
                None => c.text("Nothing playing", title, rect(x, 10.0, text_right, 30.0), pal.text),
            }

            // Top buttons sit on faint glass pills (brighter on hover); previous and next get a pill on
            // hover; play/pause is a solid disc in the text color with the glyph cut in the background's
            // color.
            for b in BUTTONS {
                let r = button_rect(b);
                let mut glyph_color = pal.text;
                if b == Button::Lyrics && lyrics_open {
                    let alpha = if pressed == Some(b) {
                        0.1
                    } else if hover == Some(b) {
                        0.2
                    } else {
                        0.14
                    };
                    c.pill(r, 8.0, gfx::with_alpha(pal.text, alpha));
                } else if b == Button::PlayPause {
                    let alpha = if pressed == Some(b) {
                        0.75
                    } else if hover == Some(b) {
                        1.0
                    } else {
                        0.9
                    };
                    c.pill(r, (r.bottom - r.top) / 2.0, gfx::with_alpha(pal.text, alpha));
                    glyph_color = pal.background;
                } else if pressed == Some(b) || hover == Some(b) {
                    let alpha = if pressed == Some(b) { 0.1 } else { 0.16 };
                    c.pill(r, 8.0, gfx::with_alpha(pal.text, alpha));
                } else if hint_text(b, lyrics_open).is_some() {
                    c.pill(r, 8.0, gfx::with_alpha(pal.text, 0.06));
                }
                let (glyph, format) = match b {
                    Button::Previous => ("\u{e892}", icons),
                    Button::PlayPause if state.is_some_and(|s| !s.paused) => ("\u{e769}", icons),
                    Button::PlayPause => ("\u{e768}", icons),
                    Button::Next => ("\u{e893}", icons),
                    Button::Lyrics => ("\u{e90b}", small),
                    Button::Back => ("\u{e8a7}", small),
                    Button::Close => ("\u{e8bb}", small),
                };
                c.text(glyph, format, r, glyph_color);
            }

            let progress = state
                .filter(|s| s.track.duration > 0.0)
                .map_or(0.0, |s| (s.position_now() / s.track.duration).clamp(0.0, 1.0) as f32);
            // A rounded track under the buttons, the played part in the text color.
            let (left, right) = (x, WIDTH - 12.0);
            c.pill(rect(left, 94.0, right, 97.0), 1.5, gfx::with_alpha(pal.text, 0.16));
            if progress > 0.0 {
                let end = (left + (right - left) * progress).max(left + 3.0);
                c.pill(rect(left, 94.0, end, 97.0), 1.5, gfx::with_alpha(pal.text, 0.85));
            }

            if lyrics_open {
                c.fill(rect(12.0, HEIGHT, WIDTH - 12.0, HEIGHT + 1.0), gfx::with_alpha(pal.text, 0.1));
                if lyrics.is_empty() {
                    let r = rect(12.0, LYRIC_TOP, WIDTH - 12.0, LYRIC_TOP + LYRIC_CURRENT);
                    c.text("No synced lyrics for this track", artist, r, gfx::with_alpha(pal.text, 0.55));
                } else {
                    // Clipped to the band, so a rolling line never runs into the controls.
                    let band = rect(0.0, HEIGHT + 1.0, WIDTH, full_height);
                    unsafe { c.target.PushAxisAlignedClip(&band, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
                    lyrics.draw(c, 12.0, LYRIC_TOP, pal.text, false);
                    unsafe { c.target.PopAxisAlignedClip() };
                }
            }

            // What the hovered top button does, on a small pill under it, last so it is on top.
            if let Some((b, label)) = hover.and_then(|b| hint_text(b, lyrics_open).map(|l| (b, l))) {
                let mut m = DWRITE_TEXT_METRICS::default();
                if let Ok(layout) = gfx::text_layout(label, hint, WIDTH, 20.0) {
                    let _ = unsafe { layout.GetMetrics(&mut m) };
                }
                let r = button_rect(b);
                let w = m.width + 16.0;
                let left = (r.left + (r.right - r.left - w) / 2.0).min(WIDTH - 4.0 - w);
                let pill = rect(left, r.bottom + 4.0, left + w, r.bottom + 24.0);
                c.pill(
                    rect(pill.left - 1.0, pill.top - 1.0, pill.right + 1.0, pill.bottom + 1.0),
                    7.0,
                    gfx::with_alpha(pal.text, 0.18),
                );
                c.pill(pill, 6.0, pal.background);
                c.pill(pill, 6.0, gfx::with_alpha(pal.text, 0.08));
                c.text(label, hint, pill, pal.text);
            }
        });
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn client_dip(hwnd: HWND, x: i32, y: i32) -> (f32, f32) {
    let scale = gfx::scale(hwnd);
    (x as f32 / scale, y as f32 / scale)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let mini = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Mini;
        if mini.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let m = &mut *mini;
        let mouse = || client_dip(hwnd, lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
        match msg {
            WM_NCHITTEST => {
                let mut p = POINT { x: lparam.0 as i16 as i32, y: (lparam.0 >> 16) as i16 as i32 };
                let _ = ScreenToClient(hwnd, &mut p);
                let (x, y) = client_dip(hwnd, p.x, p.y);
                // Anything but a button drags the window.
                return LRESULT(if hit(x, y).is_some() { HTCLIENT } else { HTCAPTION } as isize);
            }
            WM_NCMOUSEMOVE => m.set_hover(hwnd, None),
            WM_MOUSEMOVE => {
                let (x, y) = mouse();
                m.set_hover(hwnd, hit(x, y));
            }
            WM_MOUSELEAVE => {
                m.tracking = false;
                if m.hover.take().is_some() | m.pressed.take().is_some() {
                    invalidate(hwnd);
                }
            }
            WM_LBUTTONDOWN => {
                let (x, y) = mouse();
                m.pressed = hit(x, y);
                invalidate(hwnd);
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let (x, y) = mouse();
                let clicked = hit(x, y).filter(|b| m.pressed == Some(*b));
                m.pressed = None;
                invalidate(hwnd);
                // Last: closing destroys the window and frees `m`.
                match clicked {
                    Some(Button::Previous) => crate::player(&m.app, "previous"),
                    Some(Button::PlayPause) => crate::player(&m.app, "playPause"),
                    Some(Button::Next) => crate::player(&m.app, "next"),
                    Some(Button::Lyrics) => m.toggle_lyrics(hwnd),
                    Some(Button::Back) => crate::show_main(&m.app.clone()),
                    Some(Button::Close) => {
                        let _ = DestroyWindow(hwnd);
                    }
                    None => {}
                }
                return LRESULT(0);
            }
            WM_PAINT => {
                m.paint(hwnd);
                return LRESULT(0);
            }
            WM_UPDATE => {
                m.update_lyrics(hwnd);
                invalidate(hwnd);
            }
            WM_TIMER if wparam.0 == LYRIC_TIMER => {
                let _ = KillTimer(Some(hwnd), LYRIC_TIMER);
                m.lyric_timer = false;
                m.update_lyrics(hwnd);
                invalidate(hwnd);
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == TIMER => {
                invalidate(hwnd);
                m.schedule(hwnd);
                return LRESULT(0);
            }
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
                return LRESULT(0);
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER);
                let _ = KillTimer(Some(hwnd), LYRIC_TIMER);
                let mut r = RECT::default();
                if GetWindowRect(hwnd, &mut r).is_ok() {
                    *LAST_POSITION.lock().unwrap() = Some((r.left, r.top));
                }
                m.data.lock().unwrap().closed = true;
                WINDOW.store(0, Ordering::Relaxed);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(mini));
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}
