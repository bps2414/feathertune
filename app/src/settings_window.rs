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
//!   command here evaluates script in, or sends data to, the site webview. The one script Rust runs in
//!   the settings window itself is fixed text that tells its Remote section to ask again (`remote_changed`).
//! - It can only navigate within the scheme; its CSP allows our own files and IPC only.

use std::borrow::Cow;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use qrcodegen::{QrCode, QrCodeEcc};
use serde::Serialize;
use serde_json::{Value, json};

use crate::i18n::{t, tf};
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, UriSchemeContext, Url, Webview, WebviewUrl, WebviewWindowBuilder, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DwmSetWindowAttribute};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::gfx;
use crate::lastfm::LastFm;
use crate::plugins::{self, PLUGINS};
use crate::settings::{Live, valid_feature};
use crate::stats::Stats;

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

/// The What's new tab, in each language.
const CHANGELOG: &str = include_str!("../../CHANGELOG.md");
const CHANGELOG_PT_BR: &str = include_str!("../../CHANGELOG.pt-BR.md");
const LAST_VERSION: &str = "last-version.txt";
/// After an update, the version that ran before ("": one older than the record, which came in 0.2.0).
static UPDATED_FROM: OnceLock<String> = OnceLock::new();
/// What's new is still to be shown this run.
static WHATS_NEW: AtomicBool = AtomicBool::new(false);

/// Records this version in the profile and notes an update. Only the installed copy (a build run
/// from `target/` never shows it); `existed`: the profile had a `config.toml` before this start, so
/// a first install, which has none, shows nothing.
pub fn note_version(profile: &Path, existed: bool) {
    if !crate::updater::installed(profile) {
        return;
    }
    let current = env!("CARGO_PKG_VERSION");
    let path = profile.join(LAST_VERSION);
    let before = std::fs::read_to_string(&path).ok().map(|s| s.trim().to_owned());
    if before.as_deref() == Some(current) {
        return;
    }
    if let Err(e) = std::fs::write(&path, current) {
        eprintln!("{LAST_VERSION}: {e}");
    }
    let updated = match &before {
        Some(b) => crate::updater::version(current) > crate::updater::version(b),
        None => existed,
    };
    if updated {
        let _ = UPDATED_FROM.set(before.unwrap_or_default());
        WHATS_NEW.store(true, Ordering::Relaxed);
    }
}

/// Opens the window on What's new the first time the app shows after an update.
pub fn show_whats_new(app: &AppHandle) {
    if WHATS_NEW.swap(false, Ordering::Relaxed) {
        open_tab(app, "whats-new");
    }
}

/// Opens the window, or brings it to the front.
pub fn open(app: &AppHandle) {
    open_tab(app, "");
}

/// `tab`: the one to start on (a `data-tab` of index.html), only when the window is not open yet.
fn open_tab(app: &AppHandle, tab: &str) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    // Built off the main thread: creating a webview from inside an event handler can deadlock on Windows.
    let app = app.clone();
    let url: Url = format!("{ORIGIN}/#{tab}").parse().expect("static URL");
    std::thread::spawn(move || {
        let live = app.state::<Live>();
        let result = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::CustomProtocol(url))
            .title(t("feathertune settings"))
            .inner_size(740.0, 720.0)
            .min_inner_size(420.0, 400.0)
            .theme(Some(tauri::Theme::Dark))
            .data_directory(live.profile.clone())
            .additional_browser_args(&live.browser_args)
            .on_navigation(|url| url.origin().ascii_serialization() == ORIGIN)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .build();
        match result {
            // The pairing QR is only good while this window is open.
            Ok(window) => {
                let app = app.clone();
                window.on_window_event(move |event| {
                    if matches!(event, WindowEvent::Destroyed)
                        && let Some(remote) = app.try_state::<crate::remote::Remote>()
                    {
                        remote.stop_pairing();
                    }
                });
            }
            Err(e) => eprintln!("settings window: {e}"),
        }
    });
}

/// The remote changed (the pairing window, a phone asking to pair, the paired list, a connection): the
/// Remote section of the open settings window asks `settings_remote` again. Fixed script, no data in it.
pub fn remote_changed(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.eval("window.dispatchEvent(new Event('remote-changed'))");
    }
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
            name: t(p.name),
            description: t(p.description),
            cost: t(p.cost),
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
        // null: English, the page's own text.
        "strings": crate::i18n::strings(),
        "ui": {
            "language": s.ui.language,
            // The language in use (the setting may be newer, until a restart).
            "active": crate::i18n::language(),
            "languages": crate::i18n::LANGUAGES.iter().map(|(tag, name)| json!({ "tag": tag, "name": name })).collect::<Vec<_>>(),
        },
        "shortcuts": {
            "play_pause": s.shortcuts.play_pause,
            "next": s.shortcuts.next,
            "previous": s.shortcuts.previous,
            "mini_player": s.shortcuts.mini_player,
            "volume_up": s.shortcuts.volume_up,
            "volume_down": s.shortcuts.volume_down,
        },
        "discord": { "client_id": s.discord.client_id, "built_in": crate::discord::BUILT_IN },
        "lastfm": {
            "api_key": s.lastfm.api_key,
            "api_secret": s.lastfm.api_secret,
            "built_in": crate::lastfm::BUILT_IN_KEYS,
            // null: not running (plugin off or no keys at startup)
            "connected": lastfm,
        },
        "browser": { "disable_features": s.browser.disable_features },
        "remote": { "name": crate::remote::valid_name(&s.remote.name).unwrap_or_else(|| crate::remote::DEFAULT_NAME.into()) },
        "lyrics": {
            "providers": s.lyrics.providers,
            "all_providers": crate::lyrics::PROVIDERS.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
            "estimate_words": s.lyrics.estimate_words,
            "offset_ms": s.lyrics.offset_ms,
            "translate_to": s.lyrics.translate_to,
            "romanize": s.lyrics.romanize,
            "size": s.lyrics.size,
            "align": s.lyrics.align,
            "glow": s.lyrics.glow,
            "duets": s.lyrics.duets,
        },
        "audio": {
            "eq_preset": s.audio.eq_preset,
            // What the sliders show: the preset's gains, or the custom ones.
            "eq_bands": crate::audio::eq_gains(&s),
            "eq_presets": crate::audio::EQ_PRESETS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            "compressor": s.audio.compressor,
            "crossfade_ms": s.audio.crossfade_ms,
            "curve": s.audio.curve,
            "fade_in_ms": s.audio.fade_in_ms,
            "echo_out": s.audio.echo_out,
            "visualizer": s.audio.visualizer,
        },
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "desktop_lyrics": { "open": crate::desktop_lyrics::is_open(), "locked": crate::desktop_lyrics::is_locked() },
        // false: the plugin was off at startup, so nothing is recorded.
        "stats_running": app.try_state::<Stats>().is_some(),
        "restart_needed": live.restart_needed(),
        "changelog": if crate::i18n::language() == "pt-BR" { CHANGELOG_PT_BR } else { CHANGELOG },
        // null: this start was not an update.
        "updated_from": UPDATED_FROM.get(),
    }))
}

/// Changes one setting. `key` is `plugins.<id>`, `shortcuts.<name>`, `discord.client_id`,
/// `lastfm.api_key`, `lastfm.api_secret`, `browser.disable_features`, `lyrics.providers`,
/// `lyrics.estimate_words`, `lyrics.offset_ms`, `lyrics.translate_to`, `lyrics.romanize`, `lyrics.size`,
/// `lyrics.align`, `lyrics.glow`, `lyrics.duets`, `remote.name`, `audio.eq_preset`, `audio.eq_bands`,
/// `audio.compressor`, `audio.crossfade_ms`, `audio.curve`, `audio.fade_in_ms`, `audio.echo_out`, `audio.visualizer`,
/// `ui.language` or `autostart`. Returns whether a restart is needed for it to apply.
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
        ("ui", "language") => {
            let lang = value.as_str().ok_or("expected a language")?;
            if lang != "auto" && !crate::i18n::LANGUAGES.iter().any(|(tag, _)| *tag == lang) {
                return Err("unknown language".into());
            }
            live.write("ui", name, toml_edit::value(lang), None, true)?;
            Ok(true)
        }
        ("plugins", id) => {
            let plugin = plugins::get(id).ok_or("unknown plugin")?;
            let on = value.as_bool().ok_or("expected true or false")?;
            // An audio plugin is live while the page has the audio module (some audio plugin was on at startup).
            let restart = !plugin.live || (crate::audio::PLUGINS.contains(&id) && on && !crate::audio::loaded());
            live.write("plugins", id, toml_edit::value(on), plugin.legacy, restart)?;
            match id {
                "global_shortcuts" => crate::shortcuts::register(&app),
                "mini_player" if !on => crate::miniplayer::close(&app),
                "desktop_lyrics" if !on => crate::desktop_lyrics::close(&app),
                "animated_title_bar" => crate::titlebar::set_animated(&app, on),
                "navigation" => crate::titlebar::set_navigation(&app, on),
                "lyrics_translation" | "lyric_colors" | "lyric_effects" | "lyric_pulse" => {
                    crate::lyrics::republish(&app);
                    crate::desktop_lyrics::refresh();
                    crate::miniplayer::refresh();
                }
                "remote" => crate::remote::apply(&app),
                id if crate::audio::PLUGINS.contains(&id) => crate::audio::push(&app),
                _ => {}
            }
            Ok(restart)
        }
        ("shortcuts", "play_pause" | "next" | "previous" | "mini_player" | "volume_up" | "volume_down") => {
            let keys = text(&value, 64)?;
            if !keys.is_empty() {
                keys.parse::<tauri_plugin_global_shortcut::Shortcut>()
                    .map_err(|e| tf("invalid shortcut: {}", &[&e.to_string()]))?;
            }
            live.write("shortcuts", name, toml_edit::value(keys), None, false)?;
            crate::shortcuts::register(&app);
            Ok(false)
        }
        ("discord", "client_id") => {
            let id = text(&value, 32)?;
            if !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err(t("the application id is a number").into());
            }
            live.write("discord", name, toml_edit::value(id), None, true)?;
            Ok(true)
        }
        ("lastfm", "api_key" | "api_secret") => {
            let v = text(&value, 64)?;
            if !v.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(t("letters and digits only").into());
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
                    return Err(tf("invalid feature name {}", &[&format!("{f:?}")]));
                }
                names.push(f);
            }
            live.write("browser", name, toml_edit::value(names), None, true)?;
            Ok(true)
        }
        ("lyrics", "providers") => {
            let list = value.as_array().ok_or("expected a list")?;
            let mut ids = toml_edit::Array::new();
            let mut seen = Vec::new();
            for p in list {
                let p = p.as_str().ok_or("expected names")?;
                if !crate::lyrics::PROVIDERS.iter().any(|(id, _)| *id == p) || seen.contains(&p) {
                    return Err(format!("unknown lyrics source {p:?}"));
                }
                seen.push(p);
                ids.push(p);
            }
            live.write("lyrics", name, toml_edit::value(ids), None, false)?;
            Ok(false)
        }
        ("lyrics", "size" | "align" | "glow") => {
            let v = value.as_str().ok_or("expected a value")?;
            let allowed = match name {
                "size" => crate::settings::LYRICS_SIZES,
                "align" => crate::settings::LYRICS_ALIGNS,
                _ => crate::settings::LYRICS_GLOWS,
            };
            if !allowed.contains(&v) {
                return Err("unknown value".into());
            }
            live.write("lyrics", name, toml_edit::value(v), None, false)?;
            crate::lyrics::republish(&app);
            Ok(false)
        }
        ("lyrics", "estimate_words" | "romanize" | "duets") => {
            let on = value.as_bool().ok_or("expected true or false")?;
            live.write("lyrics", name, toml_edit::value(on), None, false)?;
            crate::lyrics::republish(&app);
            Ok(false)
        }
        ("lyrics", "offset_ms") => {
            let ms = value
                .as_i64()
                .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
                .filter(|ms| (-5000..=5000).contains(ms))
                .ok_or(t("a whole number of ms, from -5000 to 5000"))?;
            live.write("lyrics", name, toml_edit::value(ms), None, false)?;
            crate::lyrics::republish(&app);
            Ok(false)
        }
        ("lyrics", "translate_to") => {
            let lang = value.as_str().ok_or("expected a language")?;
            if !["auto", "off"].contains(&lang) && !crate::lyrics::TRANSLATION_LANGUAGES.iter().any(|(c, _)| *c == lang)
            {
                return Err("unknown language".into());
            }
            live.write("lyrics", name, toml_edit::value(lang), None, false)?;
            crate::lyrics::republish(&app);
            Ok(false)
        }
        ("remote", "name") => {
            // Empty: back to the default. Phones see the new name from their next connection.
            let given = text(&value, 128)?;
            let new_name = if given.is_empty() {
                crate::remote::DEFAULT_NAME.into()
            } else {
                crate::remote::valid_name(&given).ok_or(t("a name of 1 to 32 characters"))?
            };
            live.write("remote", name, toml_edit::value(new_name), None, false)?;
            crate::remote::apply(&app);
            Ok(false)
        }
        ("audio", "eq_preset") => {
            let preset = value.as_str().ok_or("expected a preset")?;
            if preset != "custom" && !crate::audio::EQ_PRESETS.iter().any(|(name, _)| *name == preset) {
                return Err("unknown preset".into());
            }
            live.write("audio", name, toml_edit::value(preset), None, false)?;
            crate::audio::push(&app);
            Ok(false)
        }
        ("audio", "eq_bands") => {
            let list = value.as_array().filter(|l| l.len() == 10).ok_or("expected 10 gains")?;
            let mut gains = toml_edit::Array::new();
            for g in list {
                let g = g.as_f64().filter(|g| g.abs() <= crate::audio::EQ_MAX_DB).ok_or("a gain from -12 to 12 dB")?;
                gains.push((g * 2.0).round() / 2.0);
            }
            live.write("audio", name, toml_edit::value(gains), None, false)?;
            crate::audio::push(&app);
            Ok(false)
        }
        ("audio", "crossfade_ms" | "fade_in_ms") => {
            let max =
                if name == "crossfade_ms" { crate::audio::MAX_CROSSFADE_MS } else { crate::audio::MAX_FADE_IN_MS };
            let ms = value
                .as_f64()
                .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
                .filter(|ms| (0.0..=f64::from(max)).contains(ms))
                .ok_or(tf("from 0 to {} ms", &[&max.to_string()]))?;
            live.write("audio", name, toml_edit::value(ms.round() as i64), None, false)?;
            crate::audio::push(&app);
            Ok(false)
        }
        ("audio", "curve") => {
            let curve = value.as_str().filter(|c| crate::audio::CURVES.contains(c)).ok_or("unknown curve")?;
            live.write("audio", name, toml_edit::value(curve), None, false)?;
            crate::audio::push(&app);
            Ok(false)
        }
        ("audio", "echo_out") => {
            let on = value.as_bool().ok_or("expected true or false")?;
            live.write("audio", name, toml_edit::value(on), None, false)?;
            crate::audio::push(&app);
            Ok(false)
        }
        ("audio", "visualizer") => {
            let style = value.as_str().filter(|v| crate::audio::VISUALIZERS.contains(v)).ok_or("unknown style")?;
            live.write("audio", name, toml_edit::value(style), None, false)?;
            crate::audio::push(&app);
            Ok(false)
        }
        ("audio", "compressor") => {
            let preset = value.as_str().filter(|p| crate::audio::COMPRESSORS.contains(p)).ok_or("unknown preset")?;
            live.write("audio", name, toml_edit::value(preset), None, false)?;
            crate::audio::push(&app);
            Ok(false)
        }
        _ => Err("unknown setting".into()),
    }
}

/// Buttons: `lastfm_connect`, `lastfm_disconnect`, `mini_player`, `desktop_lyrics` (open or close),
/// `desktop_lyrics_lock` (lock or unlock), `share_lyrics` (open), `stats_export`, `stats_clear`, `diagnostics_export`,
/// `restart`.
#[tauri::command]
pub fn settings_action(webview: Webview, app: AppHandle, action: String) -> Result<(), String> {
    guard(&webview)?;
    let lastfm = || app.try_state::<LastFm>().ok_or(t("Last.fm is not running; restart after setting it up"));
    let stats = || app.try_state::<Stats>().ok_or(t("Listening stats are not running; turn them on and restart"));
    match action.as_str() {
        "lastfm_connect" => lastfm()?.connect(&app),
        "lastfm_disconnect" => lastfm()?.disconnect(),
        "mini_player" => crate::miniplayer::open(&app),
        "desktop_lyrics" => crate::desktop_lyrics::toggle(&app),
        "desktop_lyrics_lock" => crate::desktop_lyrics::set_locked(!crate::desktop_lyrics::is_locked()),
        "share_lyrics" => crate::share_lyrics::open(&app),
        "stats_export" => {
            let file = downloads()?.join("feathertune-listening-history.csv");
            stats()?.export(&file)?;
            app.opener().reveal_item_in_dir(&file).map_err(|e| e.to_string())?;
        }
        "stats_clear" => stats()?.clear()?,
        "diagnostics_export" => {
            let file = downloads()?.join("feathertune-diagnostics.txt");
            std::fs::write(&file, crate::diag::report(&app)).map_err(|e| e.to_string())?;
            app.opener().reveal_item_in_dir(&file).map_err(|e| e.to_string())?;
        }
        // The single-instance lock is released on exit, before the new process starts.
        "restart" => {
            crate::stats::flush(&app);
            app.request_restart()
        }
        _ => return Err("unknown action".into()),
    }
    Ok(())
}

/// The Stats tab: see `Stats::summary`. `offset` is the page's time zone offset in minutes.
#[tauri::command]
pub fn settings_stats(webview: Webview, app: AppHandle, range: String, offset: i64) -> Result<Value, String> {
    guard(&webview)?;
    if !matches!(range.as_str(), "week" | "month" | "year" | "all") || !(-1440..=1440).contains(&offset) {
        return Err("invalid range".into());
    }
    let stats = app.try_state::<Stats>().ok_or(t("Listening stats are not running; turn them on and restart"))?;
    Ok(stats.summary(&range, offset))
}

const NOT_LISTENING: &str = "The phone remote is not listening, so no phone can connect. The port may be in use: \
                             change port under [remote] in config.toml and restart.";

/// The Remote section of the Experimental tab. `op`: `get`, `pair_start` (opens the pairing window and
/// returns its QR as `{ size, modules, left }`, `modules` being `size`² characters of 0 and 1, row by
/// row), `pair_stop`, `answer` ({allow, code}: the code is the one the page showed), `remove` ({key})
/// and `forget_all`. Only the settings window may call it.
#[tauri::command]
pub fn settings_remote(webview: Webview, app: AppHandle, op: String, arg: Value) -> Result<Value, String> {
    guard(&webview)?;
    let remote = app.try_state::<crate::remote::Remote>().ok_or("the phone remote is not available")?;
    match op.as_str() {
        "get" => Ok(json!({
            "listening": remote.listening().is_some(),
            // left: seconds; fresh: the QR can still be scanned.
            "pairing": remote.pairing().map(|(left, fresh)| json!({ "left": left, "fresh": fresh })),
            "pending": remote.pending(),
            "devices": remote.devices(),
        })),
        "pair_start" => {
            if remote.listening().is_none() {
                return Err(t(NOT_LISTENING).into());
            }
            let uri = remote.start_pairing().map_err(|e| match e.as_str() {
                "no local network address found" => {
                    t("This PC has no local network address. Connect it to Wi-Fi or Ethernet and try again.").into()
                }
                _ => e,
            })?;
            let qr = QrCode::encode_text(&uri, QrCodeEcc::Medium).map_err(|e| e.to_string())?;
            let size = qr.size();
            let modules: String = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .map(|(x, y)| if qr.get_module(x, y) { '1' } else { '0' })
                .collect();
            Ok(json!({ "size": size, "modules": modules, "left": remote.pairing().map_or(0, |(left, _)| left) }))
        }
        "pair_stop" => {
            remote.stop_pairing();
            Ok(Value::Null)
        }
        "answer" => {
            let allow = arg.get("allow").and_then(Value::as_bool).ok_or("expected allow or deny")?;
            let code = arg.get("code").and_then(Value::as_str).ok_or("expected the code")?;
            // Only the request the user saw: its code is the one on the screen.
            if remote.pending().is_none_or(|p| p.code != code) || !remote.answer(allow) {
                return Err(t("That request is no longer open.").into());
            }
            Ok(Value::Null)
        }
        "remove" => {
            remote.remove(arg.get("key").and_then(Value::as_str).ok_or("expected a key")?);
            Ok(Value::Null)
        }
        "forget_all" => remote.forget_all().map(|()| Value::Null),
        _ => Err("unknown operation".into()),
    }
}

/// The Extensions tab. `op`: `list`; `import` ({files: {path: base64}}, what the user picked; returns what the
/// confirmation shows); `toggle` ({id, on}; applies after a restart); `set` ({id, values}: every setting, checked
/// against the manifest; applies at once); `remove` ({id}); `open_folder`. Only the settings window may call it.
#[tauri::command]
pub fn settings_extensions(webview: Webview, app: AppHandle, op: String, arg: Value) -> Result<Value, String> {
    use crate::extensions::{Extensions, valid_id};
    use base64::Engine;

    guard(&webview)?;
    let live = app.state::<Live>();
    let extensions = app.state::<Extensions>();
    let id = || arg.get("id").and_then(Value::as_str).filter(|id| valid_id(id)).ok_or("unknown extension");
    match op.as_str() {
        "list" => Ok(extensions.view(&live.get())),
        "import" => {
            let given = arg.get("files").and_then(Value::as_object).ok_or("expected files")?;
            let mut files = std::collections::BTreeMap::new();
            for (path, data) in given {
                let data = data.as_str().ok_or("expected base64")?;
                let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
                files.insert(path.clone(), bytes);
            }
            extensions.import(files)
        }
        "toggle" => {
            let id = id()?;
            extensions.found(id).ok_or("unknown extension")?;
            let on = arg.get("on").and_then(Value::as_bool).ok_or("expected true or false")?;
            live.write("extensions", id, toml_edit::value(on), None, true)?;
            Ok(json!(true))
        }
        "set" => {
            let id = id()?;
            let ext = extensions.found(id).ok_or("unknown extension")?;
            let values = arg.get("values").and_then(Value::as_object).ok_or("expected values")?;
            let mut table = toml_edit::InlineTable::new();
            for def in &ext.settings {
                let v = values.get(&def.key).ok_or_else(|| format!("missing {}", def.key))?;
                def.check(v)?;
                table.insert(&def.key, crate::extensions::json_to_toml(v).ok_or("invalid value")?);
            }
            live.write("extension_settings", id, toml_edit::value(table), None, false)?;
            push_extension_settings(&app);
            Ok(Value::Null)
        }
        "remove" => {
            let id = id()?;
            extensions.remove(id)?;
            Ok(Value::Null)
        }
        "open_folder" => {
            let dir = extensions.dir();
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        _ => Err("unknown operation".into()),
    }
}

/// Gives the running extensions their settings of now (page and WebAssembly).
fn push_extension_settings(app: &AppHandle) {
    let live = app.state::<Live>();
    let settings = live.get();
    let all: serde_json::Map<String, Value> = app
        .state::<crate::extensions::Extensions>()
        .running()
        .iter()
        .map(|e| (e.id.clone(), Value::Object(e.values(&settings))))
        .collect();
    drop(settings);
    crate::injections::send(app, "ext", &json!({ "settings": all }));
    crate::wasm::settings(app);
}

/// The user's Downloads folder.
pub(crate) fn downloads() -> Result<std::path::PathBuf, String> {
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).map_err(|e| e.to_string())?;
        let text = path.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(path.0 as _));
        Ok(text?.into())
    }
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
