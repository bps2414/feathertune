//! Mini player: a small native window, always on top, with the artwork, title, artist, progress and
//! previous / play-pause / next, plus "back to the main window" and close. Drawn with Direct2D
//! (gfx.rs); nothing here is HTML.
//!
//! - Fed only by the `Player` state; the buttons run the same `controls` commands as the tray.
//! - Opening it hides the main window (memory Low, as in the tray); "back" shows the main window and
//!   closes it; close leaves the app in the tray. It exists only while open.
//! - The artwork URL comes from the validated state (https, one of the site's image hosts). A worker
//!   thread downloads it (no redirects, at most 4 MB) and decodes it with WIC into a small premultiplied
//!   BGRA buffer; the window turns that into a Direct2D bitmap when it paints.
//! - The worker also wakes once per second while playing to move the progress bar.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES, ID2D1Bitmap};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow, ScreenToClient,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetClientRect, GetWindowLongPtrW, GetWindowRect, HTCAPTION, HTCLIENT, IDC_ARROW, LoadCursorW, PostMessageW,
    RegisterClassExW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    WM_APP, WM_DESTROY, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY,
    WM_NCHITTEST, WM_NCMOUSEMOVE, WM_PAINT, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Result, w};

use crate::gfx::{self, ICON_FONT, TEXT_FONT, rect};
use crate::player::{Event, Player, State};
use crate::settings::Live;

/// Window size in DIPs.
const WIDTH: f32 = 340.0;
const HEIGHT: f32 = 96.0;
const ART: f32 = 80.0;
/// Decoded artwork side in pixels: the 80 DIP square at up to 200% scale.
const ART_PIXELS: u32 = 160;
const MAX_ART_BYTES: u64 = 4 * 1024 * 1024;
const WM_UPDATE: u32 = WM_APP + 1;
use crate::titlebar::WM_MOUSELEAVE;

/// The open window, 0 when closed. Only the main thread creates and destroys it.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// Where it was last closed, to reopen there.
static LAST_POSITION: Mutex<Option<(i32, i32)>> = Mutex::new(None);

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

/// Repaints it (after a theme color change).
#[allow(dead_code)] // with gfx::set_theme_color, for Phase 6
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
    art: Option<Art>,
    closed: bool,
}

struct Art {
    url: String,
    width: u32,
    height: u32,
    /// Premultiplied BGRA, `width * 4` bytes per row.
    pixels: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Previous,
    PlayPause,
    Next,
    Back,
    Close,
}

const BUTTONS: [Button; 5] = [Button::Previous, Button::PlayPause, Button::Next, Button::Back, Button::Close];

struct Mini {
    app: AppHandle,
    data: Arc<Mutex<Data>>,
    surface: gfx::Surface,
    title: IDWriteTextFormat,
    artist: IDWriteTextFormat,
    icons: IDWriteTextFormat,
    small_icons: IDWriteTextFormat,
    /// The artwork as a bitmap of the current target: URL and target generation it was made for.
    bitmap: Option<(String, u32, ID2D1Bitmap)>,
    hover: Option<Button>,
    pressed: Option<Button>,
    tracking: bool,
}

fn button_rect(b: Button) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    let x = 8.0 + ART + 8.0;
    match b {
        Button::Previous => rect(x, 54.0, x + 36.0, 86.0),
        Button::PlayPause => rect(x + 36.0, 54.0, x + 72.0, 86.0),
        Button::Next => rect(x + 72.0, 54.0, x + 108.0, 86.0),
        Button::Back => rect(WIDTH - 60.0, 4.0, WIDTH - 32.0, 32.0),
        Button::Close => rect(WIDTH - 32.0, 4.0, WIDTH - 4.0, 32.0),
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
    let mini = Box::new(Mini {
        app: app.clone(),
        data: data.clone(),
        surface: gfx::Surface::default(),
        title: font(TEXT_FONT, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        artist: font(TEXT_FONT, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        icons: font(ICON_FONT, 14.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        small_icons: font(ICON_FONT, 10.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        bitmap: None,
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
    Ok(())
}

/// Where it was last closed, or the bottom right of the main window's monitor.
unsafe fn place(app: &AppHandle, hwnd: HWND) {
    let scale = gfx::scale(hwnd);
    let (w, h) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
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
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
            )
            .user_agent(concat!("feathertune/", env!("CARGO_PKG_VERSION")))
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(15)))
            .build(),
    );
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
            let art = wanted.and_then(|url| match fetch(&agent, &url) {
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

fn fetch(agent: &ureq::Agent, url: &str) -> std::result::Result<Art, String> {
    // Same check as the page payload: https on one of the site's image hosts, nothing else.
    let url = crate::player::artwork(url).ok_or("not an artwork URL")?;
    let mut response = agent.get(&url).call().map_err(|e| e.to_string())?;
    if response.status() != 200 {
        return Err(format!("HTTP {}", response.status()));
    }
    let bytes = response.body_mut().with_config().limit(MAX_ART_BYTES).read_to_vec().map_err(|e| e.to_string())?;
    let (width, height, pixels) = decode(&bytes).map_err(|e| e.to_string())?;
    Ok(Art { url, width, height, pixels })
}

/// Any format WIC knows, scaled to fit `ART_PIXELS` and converted to premultiplied BGRA.
fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h)?;
        if w == 0 || h == 0 {
            return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
        }
        let fit = (ART_PIXELS as f32 / w.min(h) as f32).min(1.0);
        let (w, h) = (((w as f32 * fit).round() as u32).max(1), ((h as f32 * fit).round() as u32).max(1));
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&frame, w, h, WICBitmapInterpolationModeFant)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        converter.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
        Ok((w, h, pixels))
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

    fn paint(&mut self, hwnd: HWND) {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        let pal = gfx::palette();
        let data = self.data.clone();
        let data = data.lock().unwrap();
        let state = data.state.as_ref();
        let generation = self.surface.generation;
        let (hover, pressed) = (self.hover, self.pressed);
        let bitmap = &mut self.bitmap;
        let (title, artist, icons, small) = (&self.title, &self.artist, &self.icons, &self.small_icons);
        let art = data.art.as_ref().filter(|a| Some(&a.url) == state.and_then(|s| s.track.artwork.as_ref()));
        self.surface.paint(hwnd, client, |c| {
            c.fill(rect(0.0, 0.0, WIDTH, HEIGHT), pal.background);
            let art_rect = rect(8.0, 8.0, 8.0 + ART, 8.0 + ART);
            // The surface may have been rebuilt since the bitmap was made (device lost).
            let fresh = |b: &(String, u32, ID2D1Bitmap), a: &Art| b.0 == a.url && b.1 == generation;
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
                    *bitmap = unsafe { c.target.CreateBitmap(size, Some(a.pixels.as_ptr() as _), a.width * 4, &props) }
                        .ok()
                        .map(|b| (a.url.clone(), generation, b));
                }
                Some(_) => {}
                None => *bitmap = None,
            }
            match (art, bitmap.as_ref()) {
                (Some(a), Some((_, _, b))) => {
                    // Center square of the image (video thumbnails are 16:9).
                    let side = a.width.min(a.height) as f32;
                    let (x, y) = ((a.width as f32 - side) / 2.0, (a.height as f32 - side) / 2.0);
                    let source = rect(x, y, x + side, y + side);
                    unsafe {
                        c.target.DrawBitmap(
                            b,
                            Some(&art_rect),
                            1.0,
                            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                            Some(&source),
                        );
                    }
                }
                _ => {
                    c.fill(art_rect, pal.hover);
                    c.text("\u{e8d6}", icons, art_rect, gfx::with_alpha(pal.text, 0.6));
                }
            }

            let x = 8.0 + ART + 8.0;
            let text_right = button_rect(Button::Back).left - 4.0;
            match state {
                Some(s) => {
                    c.text(&s.track.title, title, rect(x, 8.0, text_right, 30.0), pal.text);
                    c.text(&s.track.artist, artist, rect(x, 30.0, WIDTH - 8.0, 50.0), gfx::with_alpha(pal.text, 0.7));
                }
                None => c.text("Nothing playing", title, rect(x, 8.0, text_right, 30.0), pal.text),
            }

            for b in BUTTONS {
                let r = button_rect(b);
                if pressed == Some(b) {
                    c.fill(r, pal.pressed);
                } else if hover == Some(b) {
                    c.fill(r, pal.hover);
                }
                let (glyph, format) = match b {
                    Button::Previous => ("\u{e892}", icons),
                    Button::PlayPause if state.is_some_and(|s| !s.paused) => ("\u{e769}", icons),
                    Button::PlayPause => ("\u{e768}", icons),
                    Button::Next => ("\u{e893}", icons),
                    Button::Back => ("\u{e73f}", small),
                    Button::Close => ("\u{e8bb}", small),
                };
                c.text(glyph, format, r, pal.text);
            }

            let progress = state
                .filter(|s| s.track.duration > 0.0)
                .map_or(0.0, |s| (s.position_now() / s.track.duration).clamp(0.0, 1.0) as f32);
            c.fill(rect(0.0, HEIGHT - 3.0, WIDTH, HEIGHT), pal.hover);
            c.fill(rect(0.0, HEIGHT - 3.0, WIDTH * progress, HEIGHT), gfx::with_alpha(pal.text, 0.8));
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
            WM_UPDATE => invalidate(hwnd),
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
