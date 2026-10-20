//! The settings window: our own HTML/CSS/JS (`app/settings/`, compiled in), served by the
//! `feathertune` URI scheme (`http://feathertune.localhost/` on Windows), never by `frontendDist`.
//! It is created when opened and destroyed when closed, so it costs nothing closed (its renderer
//! process goes away with it); it shares the site's WebView2 profile and command line, so it adds a
//! renderer, not a second browser.
//!
//! Isolation from the site (both directions):
//! - The scheme is registered for every webview (a Tauri limitation), so the handler answers 403 to
//!   any webview but `settings`: the site cannot load, embed or fetch these pages.
//! - `capabilities/settings-window.json` grants the `settings_*` commands to the webview labelled
//!   `settings` and local origins only. The site webview is `site` with a remote origin, so it never
//!   matches; `capabilities/player-bridge.json` gives the site `player_state` only. Each command also
//!   checks the caller's label itself.
//! - The settings window gets no other permission (no core window, event or webview API), and no
//!   command here evaluates script in, or sends data to, the site webview.
//! - It can only navigate within the scheme; its CSP allows our own files and IPC only.

use std::borrow::Cow;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, UriSchemeContext, Url, Webview, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_autostart::ManagerExt;

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DwmSetWindowAttribute};

use crate::gfx;
use crate::lastfm::LastFm;
use crate::plugins::{self, PLUGINS};
use crate::settings::{Live, valid_feature};

pub const SCHEME: &str = "feathertune";
const LABEL: &str = "settings";
const ORIGIN: &str = "http://feathertune.localhost";
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
                   connect-src ipc: http://ipc.localhost; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const FILES: &[(&str, &str, &[u8])] = &[
    ("/", "text/html; charset=utf-8", include_bytes!("../settings/index.html")),
    ("/settings.css", "text/css; charset=utf-8", include_bytes!("../settings/settings.css")),
    ("/settings.js", "text/javascript; charset=utf-8", include_bytes!("../settings/settings.js")),
];

/// The URI scheme handler.
pub fn protocol(ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let file = (ctx.webview_label() == LABEL && request.method() == "GET")
        .then(|| FILES.iter().find(|(path, _, _)| *path == request.uri().path()))
        .flatten();
    let builder = Response::builder()
        .header(header::CONTENT_SECURITY_POLICY, CSP)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "no-store");
    match file {
        Some((_, mime, body)) => builder.header(header::CONTENT_TYPE, *mime).body(Cow::Borrowed(*body)),
        None => builder.status(StatusCode::FORBIDDEN).body(Cow::Borrowed(&[][..])),
    }
    .expect("static response")
}

/// Opens the window, or brings it to the front.
pub fn open(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    // Built off the main thread: creating a webview from inside an event handler can deadlock on Windows.
    let app = app.clone();
    std::thread::spawn(move || {
        let live = app.state::<Live>();
        let url: Url = format!("{ORIGIN}/").parse().expect("static URL");
        let result = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::CustomProtocol(url))
            .title("feathertune settings")
            .inner_size(560.0, 720.0)
            .min_inner_size(420.0, 400.0)
            .theme(Some(tauri::Theme::Dark))
            .data_directory(live.profile.clone())
            .additional_browser_args(&live.browser_args)
            .on_navigation(|url| url.origin().ascii_serialization() == ORIGIN)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .build();
        if let Err(e) = result {
            eprintln!("settings window: {e}");
        }
    });
}

fn guard(webview: &Webview) -> Result<(), String> {
    if webview.label() == LABEL { Ok(()) } else { Err("not allowed".into()) }
}

#[derive(Serialize)]
struct PluginView {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    cost: &'static str,
    on: bool,
    live: bool,
}

/// Everything the window shows.
#[tauri::command]
pub fn settings_get(webview: Webview, app: AppHandle) -> Result<Value, String> {
    guard(&webview)?;
    let live = app.state::<Live>();
    let s = live.get();
    let plugins: Vec<PluginView> = PLUGINS
        .iter()
        .map(|p| PluginView {
            id: p.id,
            name: p.name,
            description: p.description,
            cost: p.cost,
            on: s.plugin(p.id),
            live: p.live,
        })
        .collect();
    let lastfm = app.try_state::<LastFm>().map(|l| l.connected());
    // The album theme of the title bar (null: the default look). Read again each time the window
    // gets the focus, so the frame follows the track too.
    let theme = gfx::blend().to;
    if let Ok(hwnd) = webview.window().hwnd() {
        color_frame(hwnd, theme.map(|t| t.ink));
    }
    let hex = |c: u32| format!("#{c:06x}");
    Ok(json!({
        "theme": theme.map(|t| json!({ "ink": hex(t.ink), "glows": t.glows.map(hex) })),
        "plugins": plugins,
        "shortcuts": {
            "play_pause": s.shortcuts.play_pause,
            "next": s.shortcuts.next,
            "previous": s.shortcuts.previous,
            "mini_player": s.shortcuts.mini_player,
        },
        "discord": { "client_id": s.discord.client_id },
        "lastfm": {
            "api_key": s.lastfm.api_key,
            "api_secret": s.lastfm.api_secret,
            "built_in": crate::lastfm::BUILT_IN_KEYS,
            // null: not running (plugin off or no keys at startup)
            "connected": lastfm,
        },
        "browser": { "disable_features": s.browser.disable_features },
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "restart_needed": live.restart_needed(),
    }))
}

/// Changes one setting. `key` is `plugins.<id>`, `shortcuts.<name>`, `discord.client_id`,
/// `lastfm.api_key`, `lastfm.api_secret`, `browser.disable_features` or `autostart`. Returns whether a
/// restart is needed for it to apply.
#[tauri::command]
pub fn settings_set(webview: Webview, app: AppHandle, key: String, value: Value) -> Result<bool, String> {
    guard(&webview)?;
    let live = app.state::<Live>();
    let (table, name) = key.split_once('.').unwrap_or((key.as_str(), ""));
    match (table, name) {
        ("autostart", "") => {
            let on = value.as_bool().ok_or("expected true or false")?;
            let manager = app.autolaunch();
            if on { manager.enable() } else { manager.disable() }.map_err(|e| e.to_string())?;
            crate::sync_autostart_item(&app);
            Ok(false)
        }
        ("plugins", id) => {
            let plugin = plugins::get(id).ok_or("unknown plugin")?;
            let on = value.as_bool().ok_or("expected true or false")?;
            live.write("plugins", id, toml_edit::value(on), plugin.legacy, !plugin.live)?;
            match id {
                "global_shortcuts" => crate::shortcuts::register(&app),
                "mini_player" if !on => crate::miniplayer::close(&app),
                "animated_title_bar" => crate::titlebar::set_animated(&app, on),
                _ => {}
            }
            Ok(!plugin.live)
        }
        ("shortcuts", "play_pause" | "next" | "previous" | "mini_player") => {
            let keys = text(&value, 64)?;
            if !keys.is_empty() {
                keys.parse::<tauri_plugin_global_shortcut::Shortcut>().map_err(|e| format!("invalid shortcut: {e}"))?;
            }
            live.write("shortcuts", name, toml_edit::value(keys), None, false)?;
            crate::shortcuts::register(&app);
            Ok(false)
        }
        ("discord", "client_id") => {
            let id = text(&value, 32)?;
            if !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err("the application id is a number".into());
            }
            live.write("discord", name, toml_edit::value(id), None, true)?;
            Ok(true)
        }
        ("lastfm", "api_key" | "api_secret") => {
            let v = text(&value, 64)?;
            if !v.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err("letters and digits only".into());
            }
            live.write("lastfm", name, toml_edit::value(v), None, true)?;
            Ok(true)
        }
        ("browser", "disable_features") => {
            let list = value.as_array().ok_or("expected a list")?;
            let mut names = toml_edit::Array::new();
            for f in list {
                let f = f.as_str().map(str::trim).ok_or("expected names")?;
                if !valid_feature(f) || f.len() > 100 {
                    return Err(format!("invalid feature name {f:?}"));
                }
                names.push(f);
            }
            live.write("browser", name, toml_edit::value(names), None, true)?;
            Ok(true)
        }
        _ => Err("unknown setting".into()),
    }
}

/// Buttons: `lastfm_connect`, `lastfm_disconnect`, `mini_player`, `restart`.
#[tauri::command]
pub fn settings_action(webview: Webview, app: AppHandle, action: String) -> Result<(), String> {
    guard(&webview)?;
    let lastfm = || app.try_state::<LastFm>().ok_or("Last.fm is not running; restart after setting it up");
    match action.as_str() {
        "lastfm_connect" => lastfm()?.connect(&app),
        "lastfm_disconnect" => lastfm()?.disconnect(),
        "mini_player" => crate::miniplayer::open(&app),
        // The single-instance lock is released on exit, before the new process starts.
        "restart" => app.request_restart(),
        _ => return Err("unknown action".into()),
    }
    Ok(())
}

fn text(value: &Value, max: usize) -> Result<String, String> {
    let s = value.as_str().ok_or("expected text")?.trim();
    if s.len() > max || s.chars().any(char::is_control) {
        return Err("too long or invalid".into());
    }
    Ok(s.to_owned())
}

/// Paints the system frame in the album ink with white text, or back to the default (`None`).
fn color_frame(hwnd: HWND, ink: Option<u32>) {
    // COLORREF is 0x00BBGGRR; 0xFFFFFFFF is DWMWA_COLOR_DEFAULT.
    let bgr = |c: u32| ((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff);
    let (caption, text) = ink.map_or((0xffff_ffff, 0xffff_ffff), |c| (bgr(c), 0x00ff_ffff));
    for (attribute, color) in [(DWMWA_CAPTION_COLOR, caption), (DWMWA_TEXT_COLOR, text)] {
        let color = COLORREF(color);
        unsafe {
            let _ =
                DwmSetWindowAttribute(hwnd, attribute, &color as *const _ as *const _, size_of::<COLORREF>() as u32);
        }
    }
}
