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

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, PhysicalPosition, Webview, Window, Wry};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, ICoreWebView2Controller,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, IDWriteTextFormat,
};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetClientRect, GetWindowLongPtrW, HTCAPTION, HTCLIENT, HTCLOSE, HTMAXBUTTON, HTMINBUTTON, IsZoomed,
    PostMessageW, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SIZE_MINIMIZED, SWP_NOACTIVATE, SWP_NOZORDER,
    SetWindowLongPtrW, SetWindowPos, WM_ACTIVATE, WM_CLOSE, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOVE, WM_NCACTIVATE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
    WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_PAINT, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOMMAND,
    WS_CLIPCHILDREN,
};

use crate::gfx::{self, ICON_FONT, TEXT_FONT, rect};

/// Strip height and button width, in DIPs (the Windows 11 caption metrics).
pub const HEIGHT: f32 = 32.0;
const BUTTON: f32 = 46.0;
const SUBCLASS_ID: usize = 0xfea7;
/// In the `Win32_UI_Controls` feature of `windows`, too big to enable for one constant.
pub const WM_MOUSELEAVE: u32 = 0x02a3;

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
    title: IDWriteTextFormat,
    icons: IDWriteTextFormat,
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
    site.with_webview(move |wv| {
        let hwnd = HWND(hwnd as _);
        let formats = (
            gfx::text_format(TEXT_FONT, 12.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_LEADING),
            gfx::text_format(ICON_FONT, 10.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER),
        );
        let (Ok(title), Ok(icons)) = formats else {
            eprintln!("title bar: DirectWrite unavailable");
            return;
        };
        let bar = Box::new(TitleBar {
            window,
            controller: wv.controller(),
            menu,
            surface: gfx::Surface::default(),
            title,
            icons,
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
        let text = if self.active { pal.text } else { gfx::with_alpha(pal.text, 0.45) };
        let (hover, pressed) = (self.hover, self.pressed);
        let (title, icons) = (&self.title, &self.icons);
        self.surface.paint(hwnd, area, |c| {
            c.fill(rect(0.0, 0.0, width, HEIGHT), pal.background);
            for b in [Button::Menu, Button::Minimize, Button::Maximize, Button::Close] {
                let left = button_left(b, width);
                let r = rect(left, 0.0, left + BUTTON, HEIGHT);
                let mut glyph = text;
                if hover == Some(b) || pressed == Some(b) {
                    let fill = match (b, pressed == Some(b)) {
                        (Button::Close, false) => gfx::rgb(0xc42b1c),
                        (Button::Close, true) => gfx::rgba(0xc42b1c, 0.9),
                        (_, false) => pal.hover,
                        (_, true) => pal.pressed,
                    };
                    c.fill(r, fill);
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
            let title_left = BUTTON + 4.0;
            c.text(
                "feathertune",
                title,
                rect(title_left, 0.0, button_left(Button::Minimize, width) - 8.0, HEIGHT),
                text,
            );
        });
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
            let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
            drop(Box::from_raw(data as *mut TitleBar));
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
