//! Share lyrics (plugin `share_lyrics`): a native window to make a picture card of the synced lyrics.
//! Any lines can be picked, in any number up to `MAX_PICKS`, skipping the ones in between (a skip is
//! marked on the card); the card is copied to the clipboard or saved as a PNG in Downloads.
//!
//! - The lines are the synced lyrics kept in Rust (`lyrics::Lyrics`), taken when the window opens and
//!   again whenever they change while nothing is picked. The line playing at open is picked already.
//! - Drawn with Direct2D (gfx.rs) like the mini player. The card is drawn by one function, in card
//!   DIPs: on screen scaled into the preview, and into a `gfx::Offscreen` at 3× for the picture
//!   (1080 px wide). It is opaque, so the clipboard's DIB (no alpha) and the PNG look the same.
//! - Looks: Cover (a soft field of the cover's colors: the cover cut to its center square, shrunk to
//!   5×5 and blown up again with a cubic filter, on a worker thread), Ink (the album theme's ink and
//!   glows, as the mini player), Dark, and Paper (light, tinted by the album's glows).
//! - The clipboard gets the picture as a DIB and as "PNG" (which chat apps and browsers prefer).
//! - Nothing runs while it is open but idle: it repaints on input and on a lyrics change only.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use windows::Win32::Foundation::{E_FAIL, GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_BRUSH_PROPERTIES, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_EXTEND_MODE_CLAMP, D2D1_ROUNDED_RECT, ID2D1BitmapBrush, ID2D1LinearGradientBrush,
    ID2D1RadialGradientBrush, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_LINE_SPACING_METHOD_UNIFORM,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_WRAP, IDWriteTextFormat,
    IDWriteTextLayout,
};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFOHEADER, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromWindow, ScreenToClient,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, GUID_WICPixelFormat32bppPBGRA,
    IWICImagingFactory, WICBitmapEncoderNoCache, WICBitmapInterpolationModeFant,
    WICBitmapInterpolationModeHighQualityCubic, WICRect,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STATFLAG_NONAME, STATSTG,
    STREAM_SEEK_SET,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL, VK_ESCAPE, VK_SHIFT,
};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetClientRect, GetWindowLongPtrW, HTCAPTION, HTCLIENT, ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_HAND, KillTimer,
    LoadCursorW, PostMessageW, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetCursor,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_APP, WM_DESTROY, WM_DPICHANGED,
    WM_GETICON, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY,
    WM_NCHITTEST, WM_NCMOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_SETICON, WM_TIMER, WNDCLASSEXW, WS_EX_APPWINDOW,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
};
use windows::core::{Error, Result, w};
use windows_numerics::Matrix3x2;

use crate::artwork::{self, Art};
use crate::gfx::{self, Canvas, ICON_FONT, TEXT_FONT, point, rect, rgb, rgba, with_alpha};
use crate::lyrics::{Lyrics, track_key};
use crate::player::Player;
use crate::settings::Live;
use crate::titlebar::WM_MOUSELEAVE;

/// Window size in DIPs.
const WIDTH: f32 = 880.0;
const HEIGHT: f32 = 580.0;
const HEADER: f32 = 72.0;
/// The list of lines on the left.
const LIST: D2D_RECT_F = rect(16.0, HEADER, 352.0, HEIGHT - 16.0);
const ROW: f32 = 38.0;
/// Where the card preview fits.
const PREVIEW: D2D_RECT_F = rect(LIST.right + 28.0, HEADER, WIDTH - 28.0, HEIGHT - 76.0);
const MAX_PICKS: usize = 8;

/// The card, in its own DIPs.
const CARD_W: f32 = 360.0;
const PAD: f32 = 34.0;
const ART: f32 = 56.0;
const ART_RADIUS: f32 = 12.0;
const LYRIC_SIZE: f32 = 26.0;
const LINE_GAP: f32 = 12.0;
/// The footer under the lines: a hairline and the album.
const FOOTER: f32 = 58.0;
/// The room a skip takes on the card.
const SKIP: f32 = 26.0;
/// The picture is the card at this scale: 1080 px wide.
const EXPORT_SCALE: f32 = 3.0;
/// The side of the Cover look's color field.
const FIELD: u32 = 48;

const WM_UPDATE: u32 = WM_APP + 6;
const STATUS_TIMER: usize = 0xfeb1;
const STATUS_MS: u32 = 2600;

/// The open window, 0 when closed. Only the main thread creates and destroys it.
static WINDOW: AtomicIsize = AtomicIsize::new(0);

/// Opens the window, or brings it to the front. Does nothing when the plugin is off.
pub fn open(app: &AppHandle) {
    if !app.state::<Live>().plugin("share_lyrics") {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let open = WINDOW.load(Ordering::Relaxed);
        if open != 0 {
            unsafe {
                let _ = ShowWindow(HWND(open as _), SW_SHOW);
                let _ = SetForegroundWindow(HWND(open as _));
            }
        } else if let Err(e) = unsafe { create(&handle) } {
            eprintln!("share lyrics unavailable: {e}");
        }
    });
}

/// The lyrics changed (lyrics.rs). Any thread.
pub fn refresh() {
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_UPDATE, WPARAM(0), LPARAM(0));
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Cover,
    Ink,
    Dark,
    Paper,
}

impl Look {
    fn name(self) -> &'static str {
        match self {
            Look::Cover => "Cover",
            Look::Ink => "Ink",
            Look::Dark => "Dark",
            Look::Paper => "Paper",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Close,
    Look,
    Clear,
    Save,
    Copy,
}

const BUTTONS: [Button; 5] = [Button::Close, Button::Look, Button::Clear, Button::Save, Button::Copy];

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Row(usize),
    Button(Button),
}

fn button_rect(b: Button) -> D2D_RECT_F {
    let (top, bottom) = (HEIGHT - 58.0, HEIGHT - 22.0);
    match b {
        Button::Close => rect(WIDTH - 52.0, 16.0, WIDTH - 16.0, 52.0),
        Button::Look => rect(PREVIEW.left, top, PREVIEW.left + 128.0, bottom),
        Button::Clear => rect(PREVIEW.left + 136.0, top, PREVIEW.left + 212.0, bottom),
        Button::Save => rect(WIDTH - 248.0, top, WIDTH - 156.0, bottom),
        Button::Copy => rect(WIDTH - 148.0, top, WIDTH - 28.0, bottom),
    }
}

fn inside(r: D2D_RECT_F, x: f32, y: f32) -> bool {
    (r.left..r.right).contains(&x) && (r.top..r.bottom).contains(&y)
}

/// The track and its lines when the window took them.
#[derive(Default)]
struct Snapshot {
    title: String,
    artist: String,
    album: String,
    artwork: Option<String>,
    /// The lines with text (instrumental gaps left out), with their times.
    rows: Vec<(u32, String)>,
}

/// The cover and its color field, loaded by a worker thread.
struct Media {
    url: String,
    art: Arc<Art>,
    /// `FIELD` × `FIELD`, premultiplied BGRA.
    field: Option<Vec<u8>>,
}

/// The card's text, laid out for the current picks.
struct Card {
    lines: Vec<(IDWriteTextLayout, f32)>,
    /// Where a skip mark goes.
    skips: Vec<f32>,
    /// Nothing picked: `lines` holds the hint.
    hint: bool,
    height: f32,
}

/// What the card needs on one render target, kept between frames of that target.
#[derive(Default)]
struct CardBrushes {
    generation: u32,
    url: String,
    art: Option<ID2D1BitmapBrush>,
    field: Option<ID2D1BitmapBrush>,
    shade: Option<ID2D1LinearGradientBrush>,
    spot: Option<ID2D1RadialGradientBrush>,
    /// The inner edge of the card.
    edge: Option<ID2D1SolidColorBrush>,
    glows: gfx::GlowSets,
}

struct Formats {
    heading: IDWriteTextFormat,
    small: IDWriteTextFormat,
    row: IDWriteTextFormat,
    icons: IDWriteTextFormat,
    small_icons: IDWriteTextFormat,
    button: IDWriteTextFormat,
    status: IDWriteTextFormat,
    card_title: IDWriteTextFormat,
    card_artist: IDWriteTextFormat,
    lyric: IDWriteTextFormat,
    quote: IDWriteTextFormat,
    footer: IDWriteTextFormat,
}

struct Share {
    app: AppHandle,
    surface: gfx::Surface,
    formats: Formats,
    snapshot: Snapshot,
    media: Arc<Mutex<Option<Arc<Media>>>>,
    /// Rows picked, in order.
    picks: Vec<usize>,
    /// The last row clicked, where a shift-click range starts.
    anchor: Option<usize>,
    card: Option<Card>,
    look: Look,
    /// The look was picked by hand; otherwise it follows the best one available.
    look_chosen: bool,
    brushes: CardBrushes,
    scroll: f32,
    hover: Option<Target>,
    pressed: Option<Target>,
    tracking: bool,
    status: Option<String>,
}

unsafe fn create(app: &AppHandle) -> Result<()> {
    static REGISTER: Once = Once::new();
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = w!("feathertune-share-lyrics");
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
    let font = gfx::text_format;
    let lyric =
        font("Segoe UI Variable Display", LYRIC_SIZE, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?;
    unsafe {
        lyric.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        lyric.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
        lyric.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, LYRIC_SIZE * 1.22, LYRIC_SIZE)?;
    }
    let formats = Formats {
        heading: font("Segoe UI Variable Display", 20.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        small: font(TEXT_FONT, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        row: font(TEXT_FONT, 14.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        icons: font(ICON_FONT, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        small_icons: font(ICON_FONT, 10.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        button: font(TEXT_FONT, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        status: font(TEXT_FONT, 12.5, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
        card_title: font("Segoe UI Variable Text", 16.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        card_artist: font("Segoe UI Variable Text", 13.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING)?,
        lyric,
        quote: font("Georgia", 96.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
        footer: font("Segoe UI Variable Text", 11.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_ALIGNMENT_LEADING)?,
    };
    let share = Box::new(Share {
        app: app.clone(),
        surface: gfx::Surface::default(),
        formats,
        snapshot: Snapshot::default(),
        media: Arc::default(),
        picks: Vec::new(),
        anchor: None,
        card: None,
        look: Look::Dark,
        look_chosen: false,
        brushes: CardBrushes::default(),
        scroll: 0.0,
        hover: None,
        pressed: None,
        tracking: false,
        status: None,
    });
    let share = Box::into_raw(share);
    // On success WM_NCCREATE takes the box (freed in WM_NCDESTROY).
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            class,
            w!("Share lyrics"),
            WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            Some(share as *const _),
        )?
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    unsafe {
        let s = &mut *share;
        s.take_snapshot(hwnd, true);
        s.look = s.looks()[0];
        place(app, hwnd);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            size_of_val(&corner) as u32,
        );
        // The app's icon on the taskbar button and in Alt+Tab.
        if let Some(main) = app.get_window("main").and_then(|w| w.hwnd().ok()) {
            for size in [ICON_BIG, ICON_SMALL] {
                let icon = SendMessageW(HWND(main.0), WM_GETICON, Some(WPARAM(size as usize)), None);
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(size as usize)), Some(LPARAM(icon.0)));
            }
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

/// Centered on the main window's monitor.
unsafe fn place(app: &AppHandle, hwnd: HWND) {
    let scale = gfx::scale(hwnd);
    let (w, h) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
    let main = app.get_window("main").and_then(|w| w.hwnd().ok()).map(|h| HWND(h.0)).unwrap_or(hwnd);
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTOPRIMARY), &mut info) };
    let work = info.rcWork;
    let (x, y) = (work.left + (work.right - work.left - w) / 2, work.top + (work.bottom - work.top - h) / 2);
    let _ = unsafe { SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
}

/// Loads the cover and makes its color field. Worker thread.
fn load_media(hwnd: isize, url: String, media: Arc<Mutex<Option<Arc<Media>>>>) {
    std::thread::spawn(move || {
        let art = match artwork::load(&url) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("share lyrics: artwork not loaded: {e}");
                return;
            }
        };
        let field = color_field(&art).map_err(|e| eprintln!("share lyrics: no color field: {e}")).ok();
        *media.lock().unwrap() = Some(Arc::new(Media { url, art, field }));
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_UPDATE, WPARAM(0), LPARAM(0));
        }
    });
}

/// The cover's center square shrunk to 5×5 and blown up to `FIELD` with a cubic filter: a soft field
/// of its colors, a little more saturated.
fn color_field(art: &Art) -> Result<Vec<u8>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let source = wic.CreateBitmapFromMemory(
            art.width,
            art.height,
            &GUID_WICPixelFormat32bppPBGRA,
            art.width * 4,
            &art.pixels,
        )?;
        let side = art.width.min(art.height);
        let square = WICRect {
            X: ((art.width - side) / 2) as i32,
            Y: ((art.height - side) / 2) as i32,
            Width: side as i32,
            Height: side as i32,
        };
        let clipper = wic.CreateBitmapClipper()?;
        clipper.Initialize(&source, &square)?;
        let small = wic.CreateBitmapScaler()?;
        small.Initialize(&clipper, 5, 5, WICBitmapInterpolationModeFant)?;
        let big = wic.CreateBitmapScaler()?;
        big.Initialize(&small, FIELD, FIELD, WICBitmapInterpolationModeHighQualityCubic)?;
        let mut pixels = vec![0u8; (FIELD * FIELD * 4) as usize];
        big.CopyPixels(std::ptr::null(), FIELD * 4, &mut pixels)?;
        for px in pixels.as_chunks_mut::<4>().0 {
            let gray = (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) as f32 / 3.0;
            let alpha = f32::from(px[3]);
            for c in &mut px[..3] {
                *c = (gray + (f32::from(*c) - gray) * 1.3).clamp(0.0, alpha) as u8;
            }
        }
        Ok(pixels)
    }
}

/// A brush of BGRA premultiplied pixels, smoothly scaled.
fn bitmap_brush(c: &Canvas, width: u32, height: u32, pixels: &[u8]) -> Option<ID2D1BitmapBrush> {
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    let brush = D2D1_BITMAP_BRUSH_PROPERTIES {
        extendModeX: D2D1_EXTEND_MODE_CLAMP,
        extendModeY: D2D1_EXTEND_MODE_CLAMP,
        interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    };
    unsafe {
        let size = D2D_SIZE_U { width, height };
        let bitmap = c.target.CreateBitmap(size, Some(pixels.as_ptr() as _), width * 4, &props).ok()?;
        c.target.CreateBitmapBrush(&bitmap, Some(&brush), None).ok()
    }
}

/// Moves a bitmap brush of `width` × `height` so the image covers `r`, centered.
fn cover(brush: &ID2D1BitmapBrush, width: u32, height: u32, r: D2D_RECT_F) {
    let (w, h) = (width as f32, height as f32);
    let k = ((r.right - r.left) / w).max((r.bottom - r.top) / h);
    let x = r.left + ((r.right - r.left) - w * k) / 2.0;
    let y = r.top + ((r.bottom - r.top) - h * k) / 2.0;
    unsafe { brush.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x, M32: y }) };
}

impl CardBrushes {
    /// Rebuilds what belongs to another target or another cover.
    fn sync(&mut self, c: &Canvas, media: Option<&Media>) {
        if self.generation != c.generation {
            self.generation = c.generation;
            self.url.clear();
            self.shade = c.linear(point(0.0, 0.0), point(0.0, 1.0), rgba(0, 0.28), rgba(0, 0.62));
            self.spot = c.radial(rgb(0xffffff));
            self.edge = c.solid(rgb(0xffffff));
        }
        let url = media.map_or("", |m| m.url.as_str());
        if self.url != url {
            self.url = url.to_owned();
            self.art = media.and_then(|m| bitmap_brush(c, m.art.width, m.art.height, &m.art.pixels));
            self.field = media.and_then(|m| m.field.as_ref()).and_then(|f| bitmap_brush(c, FIELD, FIELD, f));
        }
    }
}

/// The glows of a theme on the card: center, radii (fractions of the card width) and strength.
fn card_glows(h: f32) -> [(f32, f32, f32, f32, f32); 4] {
    [
        (CARD_W * 0.95, 0.0, 0.95, 0.7, 0.6),
        (0.0, h * 0.72, 0.8, 0.75, 0.5),
        (CARD_W, h, 0.7, 0.55, 0.45),
        (CARD_W * 0.3, h * 0.3, 0.55, 0.45, 0.22),
    ]
}

/// Draws the card at the origin, `CARD_W` × `card.height` DIPs.
fn draw_card(
    c: &Canvas,
    card: &Card,
    look: Look,
    snap: &Snapshot,
    media: Option<&Media>,
    f: &Formats,
    b: &mut CardBrushes,
) {
    b.sync(c, media);
    let h = card.height;
    let full = rect(0.0, 0.0, CARD_W, h);
    let theme = gfx::blend().to;
    let ink = theme.map_or(0x161616, |t| t.ink);
    let paper = look == Look::Paper;
    let text = if paper { rgb(0x1d1b18) } else { rgb(0xffffff) };
    let sub = with_alpha(text, if paper { 0.6 } else { 0.68 });
    let glows = |b: &mut CardBrushes, strength: f32| {
        let Some(theme) = theme else { return };
        b.glows.prepare(c, &[theme]);
        let Some(set) = b.glows.get(&theme) else { return };
        for (brush, (x, y, rx, ry, alpha)) in set.iter().zip(card_glows(h)) {
            unsafe {
                brush.SetCenter(point(x, y));
                brush.SetRadiusX(rx * CARD_W);
                brush.SetRadiusY(ry * CARD_W);
                brush.SetOpacity(alpha * strength);
            }
            c.fill_with(full, brush);
        }
    };
    match look {
        Look::Cover => {
            c.fill(full, rgb(ink));
            if let Some(field) = &b.field {
                cover(field, FIELD, FIELD, full);
                c.fill_with(full, field);
            }
            glows(b, 0.4);
            // Darker toward the bottom, where most of the text is.
            if let Some(shade) = &b.shade {
                unsafe { shade.SetEndPoint(point(0.0, h)) };
                c.fill_with(full, shade);
            }
        }
        Look::Ink => {
            c.fill(full, rgb(ink));
            glows(b, 1.0);
        }
        Look::Dark => {
            c.fill(full, rgb(0x121212));
            glows(b, 0.35);
            if let Some(spot) = &b.spot {
                unsafe {
                    spot.SetCenter(point(CARD_W * 0.2, 0.0));
                    spot.SetRadiusX(CARD_W * 0.9);
                    spot.SetRadiusY(CARD_W * 0.6);
                    spot.SetOpacity(0.07);
                }
                c.fill_with(full, spot);
            }
        }
        Look::Paper => {
            c.fill(full, rgb(0xf6f2eb));
            glows(b, 0.28);
        }
    }

    // A soft sheen from the top left, and a big quote mark behind the header.
    if let Some(spot) = &b.spot {
        unsafe {
            spot.SetCenter(point(0.0, 0.0));
            spot.SetRadiusX(CARD_W * 1.1);
            spot.SetRadiusY(CARD_W * 0.8);
            spot.SetOpacity(if paper { 0.35 } else { 0.06 });
        }
        c.fill_with(full, spot);
    }
    c.text(
        "\u{201d}",
        &f.quote,
        rect(CARD_W - 140.0, PAD - 34.0, CARD_W - PAD + 6.0, PAD + 76.0),
        with_alpha(text, 0.14),
    );

    // Header: the cover on a soft shadow, the title and the artist.
    let art = rect(PAD, PAD, PAD + ART, PAD + ART);
    for (grow, alpha) in [(10.0, 0.04), (5.0, 0.07), (2.0, 0.12)] {
        let shadow = rect(art.left - grow, art.top - grow + 4.0, art.right + grow, art.bottom + grow + 4.0);
        c.pill(shadow, ART_RADIUS + grow, rgba(0, if paper { alpha * 0.6 } else { alpha * 1.6 }));
    }
    match (&b.art, media) {
        (Some(brush), Some(m)) => unsafe {
            cover(brush, m.art.width, m.art.height, art);
            let rounded = D2D1_ROUNDED_RECT { rect: art, radiusX: ART_RADIUS, radiusY: ART_RADIUS };
            c.target.FillRoundedRectangle(&rounded, brush);
        },
        _ => {
            c.pill(art, ART_RADIUS, with_alpha(text, 0.12));
            c.text("\u{e8d6}", &f.icons, art, sub);
        }
    }
    let x = art.right + 16.0;
    let right = CARD_W - PAD - 40.0;
    c.text(&snap.title, &f.card_title, rect(x, PAD + 6.0, right, PAD + 30.0), text);
    c.text(&snap.artist, &f.card_artist, rect(x, PAD + 30.0, right, PAD + 50.0), sub);

    // The lines; a skip is three small dots.
    for (layout, y) in &card.lines {
        c.layout(layout, point(PAD, *y), if card.hint { with_alpha(text, 0.45) } else { text });
    }
    for y in &card.skips {
        for i in 0..3 {
            let dot = PAD + 2.0 + i as f32 * 10.0;
            c.pill(rect(dot, y + SKIP / 2.0 - 2.5, dot + 5.0, y + SKIP / 2.0 + 2.5), 2.5, with_alpha(text, 0.4));
        }
    }

    // Footer: a hairline, a note and the album (or the artist).
    let y = h - PAD - FOOTER + 26.0;
    c.fill(rect(PAD, y, CARD_W - PAD, y + 1.0), with_alpha(text, 0.16));
    let label = if snap.album.is_empty() { &snap.artist } else { &snap.album };
    c.text("\u{e8d6}", &f.small_icons, rect(PAD, y + 16.0, PAD + 12.0, y + 32.0), sub);
    c.text(label, &f.footer, rect(PAD + 20.0, y + 16.0, CARD_W - PAD, y + 32.0), sub);

    // A faint inner edge, like glass.
    if let Some(edge) = &b.edge {
        let inset = rect(10.0, 10.0, CARD_W - 10.0, h - 10.0);
        let rounded = D2D1_ROUNDED_RECT { rect: inset, radiusX: 18.0, radiusY: 18.0 };
        unsafe {
            edge.SetColor(&with_alpha(text, if paper { 0.1 } else { 0.12 }));
            c.target.DrawRoundedRectangle(&rounded, edge, 1.0, None);
        }
    }
}

fn layout_card(rows: &[(u32, String)], picks: &[usize], lyric: &IDWriteTextFormat) -> Result<Card> {
    let width = CARD_W - 2.0 * PAD;
    let hint = picks.is_empty();
    let texts: Vec<&str> =
        if hint { vec!["Pick lines on the left"] } else { picks.iter().map(|i| rows[*i].1.as_str()).collect() };
    let mut y = PAD + ART + 34.0;
    let (mut lines, mut skips) = (Vec::new(), Vec::new());
    for (n, text) in texts.iter().enumerate() {
        if n > 0 && picks[n] > picks[n - 1] + 1 {
            skips.push(y - LINE_GAP / 2.0);
            y += SKIP;
        }
        let layout = gfx::text_layout(text, lyric, width, 2000.0)?;
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m)? };
        lines.push((layout, y));
        y += m.height + LINE_GAP;
    }
    Ok(Card { lines, skips, hint, height: y - LINE_GAP + FOOTER + PAD })
}

impl Share {
    /// Takes the current track and its lines; `pick_now` picks the line playing.
    fn take_snapshot(&mut self, hwnd: HWND, pick_now: bool) {
        let state = self.app.state::<Player>().current();
        let synced = self.app.state::<Lyrics>().current();
        let mut snap = Snapshot::default();
        if let Some(s) = &state {
            snap.title = s.track.title.clone();
            snap.artist = s.track.artist.clone();
            snap.album = s.track.album.clone();
            snap.artwork = s.track.artwork.clone();
            if let Some(synced) = synced.filter(|l| l.key == track_key(&s.track)) {
                snap.rows = synced
                    .lines
                    .iter()
                    .filter(|l| !l.text.trim().is_empty())
                    .map(|l| (l.ms, l.text.trim().to_owned()))
                    .collect();
            }
        }
        self.picks.clear();
        self.anchor = None;
        self.scroll = 0.0;
        if let Some(s) = state.as_ref().filter(|_| pick_now)
            && let Some(now) = snap.rows.iter().rposition(|(ms, _)| f64::from(*ms) <= s.position_now() * 1000.0)
        {
            self.picks.push(now);
            self.anchor = Some(now);
            let list = LIST.bottom - LIST.top - 12.0;
            self.scroll = (now as f32 * ROW - list / 2.0 + ROW / 2.0).clamp(0.0, max_scroll(snap.rows.len()));
        }
        let url = snap.artwork.clone();
        let cached = self.media.lock().unwrap().as_ref().is_some_and(|m| Some(&m.url) == url.as_ref());
        if !cached {
            *self.media.lock().unwrap() = None;
            if let Some(url) = url {
                load_media(hwnd.0 as isize, url, self.media.clone());
            }
        }
        self.snapshot = snap;
        self.card = None;
    }

    /// The looks that can be drawn now: Cover needs the cover, Ink the album theme.
    fn looks(&self) -> Vec<Look> {
        let mut looks = Vec::new();
        if self.media.lock().unwrap().is_some() {
            looks.push(Look::Cover);
        }
        if gfx::blend().to.is_some() {
            looks.push(Look::Ink);
        }
        looks.extend([Look::Dark, Look::Paper]);
        looks
    }

    fn card(&mut self) -> Option<&Card> {
        if self.card.is_none() {
            match layout_card(&self.snapshot.rows, &self.picks, &self.formats.lyric) {
                Ok(card) => self.card = Some(card),
                Err(e) => eprintln!("share lyrics: layout failed: {e}"),
            }
        }
        self.card.as_ref()
    }

    fn hit(&self, x: f32, y: f32) -> Option<Target> {
        if let Some(b) = BUTTONS.into_iter().find(|b| inside(button_rect(*b), x, y)) {
            return Some(Target::Button(b));
        }
        if !inside(rect(LIST.left + 6.0, LIST.top + 6.0, LIST.right - 6.0, LIST.bottom - 6.0), x, y) {
            return None;
        }
        let i = ((y - LIST.top - 6.0 + self.scroll) / ROW) as usize;
        (i < self.snapshot.rows.len()).then_some(Target::Row(i))
    }

    fn set_hover(&mut self, hwnd: HWND, hover: Option<Target>) {
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

    fn toggle_row(&mut self, hwnd: HWND, i: usize, range: bool) {
        let wanted: Vec<usize> = match (range, self.anchor) {
            (true, Some(a)) => (a.min(i)..=a.max(i)).collect(),
            _ => vec![i],
        };
        if !range && self.picks.contains(&i) {
            self.picks.retain(|p| *p != i);
        } else {
            for p in wanted {
                if !self.picks.contains(&p) {
                    if self.picks.len() == MAX_PICKS {
                        self.set_status(hwnd, format!("Up to {MAX_PICKS} lines fit on a card"));
                        break;
                    }
                    self.picks.push(p);
                }
            }
            self.picks.sort_unstable();
        }
        self.anchor = Some(i);
        self.card = None;
    }

    fn set_status(&mut self, hwnd: HWND, status: String) {
        self.status = Some(status);
        unsafe { SetTimer(Some(hwnd), STATUS_TIMER, STATUS_MS, None) };
    }

    /// The card as a picture: its pixels (top-down, opaque BGRA), width and height.
    fn render(&mut self) -> Result<(Vec<u8>, u32, u32)> {
        let look = self.look;
        let media = self.media.lock().unwrap().clone();
        self.card();
        let card = self.card.as_ref().ok_or_else(|| Error::from_hresult(E_FAIL))?;
        let (w, h) = ((CARD_W * EXPORT_SCALE).round() as i32, (card.height * EXPORT_SCALE).round() as i32);
        let mut picture = gfx::Offscreen::new(w, h)?;
        let mut brushes = CardBrushes::default();
        let (snap, formats) = (&self.snapshot, &self.formats);
        picture.draw(EXPORT_SCALE, |c| draw_card(c, card, look, snap, media.as_deref(), formats, &mut brushes))?;
        Ok((picture.pixels().to_vec(), w as u32, h as u32))
    }

    fn copy(&mut self, hwnd: HWND) -> std::result::Result<(), String> {
        let (pixels, w, h) = self.render().map_err(|e| e.to_string())?;
        let png = png(&pixels, w, h).map_err(|e| e.to_string())?;
        // CF_DIB: a header and the rows bottom-up.
        let header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: h as i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: w * h * 4,
            ..Default::default()
        };
        let mut dib = Vec::with_capacity(size_of::<BITMAPINFOHEADER>() + pixels.len());
        dib.extend_from_slice(unsafe {
            std::slice::from_raw_parts((&header as *const BITMAPINFOHEADER).cast::<u8>(), size_of::<BITMAPINFOHEADER>())
        });
        for row in pixels.chunks_exact(w as usize * 4).rev() {
            dib.extend_from_slice(row);
        }
        unsafe {
            OpenClipboard(Some(hwnd)).map_err(|e| e.to_string())?;
            let result = EmptyClipboard().map_err(|e| e.to_string()).and_then(|()| {
                const CF_DIB: u32 = 8;
                clipboard_set(CF_DIB, &dib)?;
                // Optional: apps that read it get the exact picture.
                let _ = clipboard_set(RegisterClipboardFormatW(w!("PNG")), &png);
                Ok(())
            });
            let _ = CloseClipboard();
            result
        }
    }

    fn save(&mut self) -> std::result::Result<std::path::PathBuf, String> {
        let (pixels, w, h) = self.render().map_err(|e| e.to_string())?;
        let png = png(&pixels, w, h).map_err(|e| e.to_string())?;
        let dir = crate::settings_window::downloads()?;
        let base = file_name(&self.snapshot.artist, &self.snapshot.title);
        let mut file = dir.join(format!("{base}.png"));
        let mut n = 2;
        while file.exists() {
            file = dir.join(format!("{base} ({n}).png"));
            n += 1;
        }
        std::fs::write(&file, png).map_err(|e| e.to_string())?;
        Ok(file)
    }

    fn paint(&mut self, hwnd: HWND) {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        let pal = gfx::palette();
        let media = self.media.lock().unwrap().clone();
        let look = self.look;
        let has_card = self.card().is_some();
        let Share { surface, formats: f, snapshot: snap, picks, card, brushes, scroll, hover, pressed, status, .. } =
            self;
        let (hover, pressed, scroll) = (*hover, *pressed, *scroll);
        let text = pal.text;
        let sub = with_alpha(text, 0.62);
        surface.paint(hwnd, client, |c| {
            c.fill(rect(0.0, 0.0, WIDTH, HEIGHT), pal.background);

            // Header.
            c.text("Share lyrics", &f.heading, rect(24.0, 14.0, 400.0, 40.0), text);
            let hint = if snap.rows.is_empty() {
                "No synced lyrics for this track"
            } else {
                "Pick any lines, skip the rest. Shift-click picks a range."
            };
            c.text(hint, &f.small, rect(24.0, 40.0, 560.0, 58.0), sub);
            if let Some(s) = status {
                c.text(s, &f.status, rect(400.0, 26.0, button_rect(Button::Close).left - 12.0, 44.0), sub);
            }

            // The list.
            c.pill(LIST, 12.0, with_alpha(text, 0.045));
            let clip = rect(LIST.left + 6.0, LIST.top + 6.0, LIST.right - 6.0, LIST.bottom - 6.0);
            unsafe { c.target.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
            if snap.rows.is_empty() {
                let format = &f.small;
                c.text("Lines appear here when LRCLIB has synced lyrics.", format, rect(30.0, 90.0, 340.0, 110.0), sub);
            }
            let first = (scroll / ROW) as usize;
            let visible = ((clip.bottom - clip.top) / ROW) as usize + 2;
            for i in first..(first + visible).min(snap.rows.len()) {
                let top = clip.top + i as f32 * ROW - scroll;
                let r = rect(clip.left, top + 1.0, clip.right, top + ROW - 1.0);
                let picked = picks.contains(&i);
                let over = hover == Some(Target::Row(i));
                if picked {
                    c.pill(r, 9.0, with_alpha(text, if over { 0.17 } else { 0.13 }));
                } else if over {
                    c.pill(r, 9.0, with_alpha(text, 0.06));
                }
                // A check disc when picked, a ring otherwise.
                let mark = rect(r.left + 12.0, top + ROW / 2.0 - 9.0, r.left + 30.0, top + ROW / 2.0 + 9.0);
                if picked {
                    c.pill(mark, 9.0, text);
                    c.text("\u{e73e}", &f.small_icons, mark, pal.background);
                } else {
                    c.pill(mark, 9.0, with_alpha(text, if over { 0.45 } else { 0.28 }));
                    let inner = rect(mark.left + 1.5, mark.top + 1.5, mark.right - 1.5, mark.bottom - 1.5);
                    c.pill(inner, 7.5, pal.background);
                    c.pill(inner, 7.5, with_alpha(text, 0.045));
                }
                let color = if picked { text } else { with_alpha(text, 0.82) };
                c.text(&snap.rows[i].1, &f.row, rect(r.left + 42.0, r.top, r.right - 10.0, r.bottom), color);
            }
            unsafe { c.target.PopAxisAlignedClip() };

            // The card, scaled into the preview on a soft shadow.
            if let Some(card) = card.as_ref().filter(|_| has_card) {
                let (aw, ah) = (PREVIEW.right - PREVIEW.left, PREVIEW.bottom - PREVIEW.top);
                let k = (aw / CARD_W).min(ah / card.height).min(1.0);
                let (cw, ch) = (CARD_W * k, card.height * k);
                let (x, y) = (PREVIEW.left + (aw - cw) / 2.0, PREVIEW.top + (ah - ch) / 2.0);
                for (grow, alpha) in [(22.0, 0.04), (12.0, 0.07), (5.0, 0.1), (1.5, 0.14)] {
                    let shadow = rect(x - grow, y - grow + 8.0, x + cw + grow, y + ch + grow + 8.0);
                    c.pill(shadow, 4.0 + grow, rgba(0, alpha));
                }
                unsafe {
                    c.target.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x, M32: y });
                    c.target
                        .PushAxisAlignedClip(&rect(0.0, 0.0, CARD_W, card.height), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                }
                draw_card(c, card, look, snap, media.as_deref(), f, brushes);
                unsafe {
                    c.target.PopAxisAlignedClip();
                    c.target.SetTransform(&Matrix3x2::identity());
                }
            }

            // Buttons; Copy is the solid one, Copy and Save are dim with nothing picked.
            let empty = picks.is_empty();
            for b in BUTTONS {
                let r = button_rect(b);
                let state = if pressed == Some(Target::Button(b)) {
                    2
                } else if hover == Some(Target::Button(b)) {
                    1
                } else {
                    0
                };
                let dim = empty && matches!(b, Button::Copy | Button::Save | Button::Clear);
                let label = match b {
                    Button::Close => {
                        if state > 0 {
                            c.pill(r, 10.0, with_alpha(text, 0.1));
                        }
                        c.text("\u{e8bb}", &f.small_icons, r, text);
                        continue;
                    }
                    Button::Look => format!("Look: {}  \u{25be}", look.name()),
                    Button::Clear => "Clear".into(),
                    Button::Save => "Save PNG".into(),
                    Button::Copy => "Copy image".into(),
                };
                let radius = (r.bottom - r.top) / 2.0;
                if b == Button::Copy {
                    let alpha = if dim { 0.35 } else { [0.92, 1.0, 0.78][state] };
                    c.pill(r, radius, with_alpha(text, alpha));
                    c.text(&label, &f.button, r, pal.background);
                } else {
                    let alpha = if dim { 0.05 } else { [0.08, 0.13, 0.06][state] };
                    c.pill(r, radius, with_alpha(text, alpha));
                    c.text(&label, &f.button, r, with_alpha(text, if dim { 0.4 } else { 1.0 }));
                }
            }
        });
    }
}

fn max_scroll(rows: usize) -> f32 {
    (rows as f32 * ROW - (LIST.bottom - LIST.top - 12.0)).max(0.0)
}

/// `artist - title`, safe as a file name.
fn file_name(artist: &str, title: &str) -> String {
    let raw = if artist.is_empty() { title.to_owned() } else { format!("{artist} - {title}") };
    let clean: String =
        raw.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).take(80).collect();
    let clean = clean.trim().trim_end_matches('.');
    if clean.is_empty() { "lyrics".into() } else { format!("{clean} (lyrics)") }
}

/// Encodes opaque top-down BGRA pixels as PNG.
fn png(pixels: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = SHCreateMemStream(None).ok_or_else(|| Error::from_hresult(E_FAIL))?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let (mut frame, mut options) = (None, None);
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.ok_or_else(|| Error::from_hresult(E_FAIL))?;
        frame.Initialize(options.as_ref())?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        if format != GUID_WICPixelFormat32bppBGRA {
            return Err(Error::from_hresult(E_FAIL));
        }
        frame.WritePixels(h, w * 4, pixels)?;
        frame.Commit()?;
        encoder.Commit()?;
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut bytes = vec![0u8; stat.cbSize as usize];
        let mut read = 0u32;
        stream.Read(bytes.as_mut_ptr().cast(), bytes.len() as u32, Some(&mut read)).ok()?;
        bytes.truncate(read as usize);
        Ok(bytes)
    }
}

/// Puts `bytes` on the open clipboard as `format`.
unsafe fn clipboard_set(format: u32, bytes: &[u8]) -> std::result::Result<(), String> {
    unsafe {
        let memory: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.to_string())?;
        let target = GlobalLock(memory);
        if target.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err("out of memory".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast(), bytes.len());
        let _ = GlobalUnlock(memory);
        // On success the clipboard owns the memory.
        if let Err(e) = SetClipboardData(format, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(e.to_string());
        }
        Ok(())
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

fn key_down(key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(i32::from(key.0)) < 0 }
}

/// Runs a button. Close destroys the window and frees `s`.
fn press(s: &mut Share, hwnd: HWND, b: Button) {
    match b {
        Button::Close => {
            let _ = unsafe { DestroyWindow(hwnd) };
            return;
        }
        Button::Look => {
            let looks = s.looks();
            let i = looks.iter().position(|l| *l == s.look).map_or(0, |i| (i + 1) % looks.len());
            s.look = looks[i];
            s.look_chosen = true;
        }
        Button::Clear => {
            s.picks.clear();
            s.anchor = None;
            s.card = None;
        }
        _ if s.picks.is_empty() => {}
        Button::Copy => {
            let status = match s.copy(hwnd) {
                Ok(()) => "Copied. Paste it anywhere".to_owned(),
                Err(e) => format!("Not copied: {e}"),
            };
            s.set_status(hwnd, status);
        }
        Button::Save => {
            let status = match s.save() {
                Ok(file) => {
                    let _ = s.app.opener().reveal_item_in_dir(&file);
                    "Saved to Downloads".to_owned()
                }
                Err(e) => format!("Not saved: {e}"),
            };
            s.set_status(hwnd, status);
        }
    }
    invalidate(hwnd);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let share = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Share;
        if share.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let s = &mut *share;
        let mouse = || client_dip(hwnd, lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
        match msg {
            WM_NCHITTEST => {
                let mut p = POINT { x: lparam.0 as i16 as i32, y: (lparam.0 >> 16) as i16 as i32 };
                let _ = ScreenToClient(hwnd, &mut p);
                let (x, y) = client_dip(hwnd, p.x, p.y);
                // The header drags the window.
                let drag = y < HEADER - 8.0 && !inside(button_rect(Button::Close), x, y);
                return LRESULT(if drag { HTCAPTION } else { HTCLIENT } as isize);
            }
            WM_SETCURSOR if s.hover.is_some() => {
                SetCursor(LoadCursorW(None, IDC_HAND).ok());
                return LRESULT(1);
            }
            WM_NCMOUSEMOVE => s.set_hover(hwnd, None),
            WM_MOUSEMOVE => {
                let (x, y) = mouse();
                let hover = s.hit(x, y);
                s.set_hover(hwnd, hover);
            }
            WM_MOUSELEAVE => {
                s.tracking = false;
                if s.hover.take().is_some() | s.pressed.take().is_some() {
                    invalidate(hwnd);
                }
            }
            WM_MOUSEWHEEL => {
                let delta = f32::from((wparam.0 >> 16) as u16 as i16) / 120.0;
                let scroll = (s.scroll - delta * ROW * 2.0).clamp(0.0, max_scroll(s.snapshot.rows.len()));
                if scroll != s.scroll {
                    s.scroll = scroll;
                    invalidate(hwnd);
                }
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = mouse();
                s.pressed = s.hit(x, y);
                invalidate(hwnd);
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let (x, y) = mouse();
                let clicked = s.hit(x, y).filter(|t| s.pressed == Some(*t));
                s.pressed = None;
                invalidate(hwnd);
                match clicked {
                    Some(Target::Row(i)) => s.toggle_row(hwnd, i, key_down(VK_SHIFT)),
                    // Last: closing destroys the window and frees `s`.
                    Some(Target::Button(b)) => press(s, hwnd, b),
                    None => {}
                }
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                let ctrl = key_down(VK_CONTROL);
                match wparam.0 as u16 {
                    k if k == VK_ESCAPE.0 => press(s, hwnd, Button::Close),
                    0x43 if ctrl => press(s, hwnd, Button::Copy),
                    0x53 if ctrl => press(s, hwnd, Button::Save),
                    _ => {}
                }
                return LRESULT(0);
            }
            WM_PAINT => {
                s.paint(hwnd);
                return LRESULT(0);
            }
            WM_UPDATE => {
                // New lines while nothing is picked: show the new ones.
                let rows_changed = s.picks.is_empty() && {
                    let state = s.app.state::<Player>().current();
                    let synced = s.app.state::<Lyrics>().current();
                    let count = state
                        .zip(synced)
                        .filter(|(st, l)| l.key == track_key(&st.track))
                        .map_or(0, |(_, l)| l.lines.iter().filter(|l| !l.text.trim().is_empty()).count());
                    count != s.snapshot.rows.len()
                        || s.app.state::<Player>().current().is_some_and(|st| st.track.title != s.snapshot.title)
                };
                if rows_changed {
                    s.take_snapshot(hwnd, false);
                }
                if !s.look_chosen || !s.looks().contains(&s.look) {
                    s.look = s.looks()[0];
                }
                invalidate(hwnd);
            }
            WM_TIMER if wparam.0 == STATUS_TIMER => {
                let _ = KillTimer(Some(hwnd), STATUS_TIMER);
                s.status = None;
                invalidate(hwnd);
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
                let _ = KillTimer(Some(hwnd), STATUS_TIMER);
                WINDOW.store(0, Ordering::Relaxed);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(share));
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("A/B", "C: D?"), "A_B - C_ D_ (lyrics)");
        assert_eq!(file_name("", "..."), "lyrics");
    }
}
