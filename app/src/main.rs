// feathertune: the official web music player in a lean WebView2 window, with a tray icon.
// Usage: feathertune [--measure] [--hidden] [--mini-player] [start url on the player domain]
// --hidden starts in the tray (the autostart entry passes it). --mini-player starts with the mini
// player instead of the main window, or opens it in the running instance.
// The main window has no system frame: titlebar.rs draws the title bar and the site is a child
// webview (label `site`) below it.
// --measure prints the time until the player bar shows up and, every 10 s, the <video> time and
// login state, and the audio graph's state, so measure.ps1 can record them. The page can call two Tauri
// commands, `player_state` (player.rs) and `audio_facts` (audio_facts.rs), which only report data
// (see capabilities/).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod album_color;
mod artwork;
mod audio;
mod audio_facts;
mod desktop_lyrics;
mod discord;
mod gfx;
mod injections;
mod lastfm;
mod lyric_view;
mod lyrics;
mod miniplayer;
mod notify;
mod player;
mod plugins;
mod privacy;
mod settings;
mod settings_window;
mod share_lyrics;
mod shortcuts;
mod smtc;
mod splash;
mod stats;
mod taskbar;
mod titlebar;
mod traymenu;
mod updater;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

const START_URL: &str = "https://music.youtube.com";
// wry's defaults (dropped once we pass our own args), Chromium's media keys and flyout entry (ours
// is in smtc.rs). Keep every feature in this single --disable-features: Chromium only honors the
// last one. Features that the measured experiments kept go here (MediaRouter, DialMediaRouteProvider:
// no Cast discovery); the ones still being measured come from `[browser] disable_features` in
// config.toml (see `browser_args`).
const DISABLED_FEATURES: &[&str] = &[
    "msWebOOUI",
    "msPdfOOUI",
    "msSmartScreenProtection",
    "HardwareMediaKeyHandling",
    "MediaRouter",
    "DialMediaRouteProvider",
];

const BAR_PROBE: &str = "(() => { const b = document.querySelector('ytmusic-player-bar'); \
    return !!b && b.getBoundingClientRect().height > 0 && !!b.querySelector('.title')?.textContent.trim() })()";
const STATUS_PROBE: &str = "(() => { const v = document.querySelector('video'); \
    return { time: v ? Math.round(v.currentTime) : null, paused: v ? v.paused : null, \
    loggedIn: window.ytcfg?.get?.('LOGGED_IN') ?? null } })()";

fn main() {
    let start = Instant::now();
    let mut measure = false;
    let mut hidden = false;
    let mut mini = false;
    let mut url: Url = START_URL.parse().unwrap();
    for arg in std::env::args().skip(1) {
        if arg == "--measure" {
            measure = true;
        } else if arg == "--hidden" {
            hidden = true;
        } else if arg == "--mini-player" {
            mini = true;
        } else if let Ok(u) = arg.parse::<Url>()
            && u.scheme() == "https"
            && u.host_str() == Some("music.youtube.com")
        {
            url = u;
        }
    }
    smtc::set_app_id();
    let profile = profile_dir();
    let settings = settings::Settings::load(&profile);
    let args = browser_args(&settings.browser);
    let script = injections::script(&settings);
    // The start screen (splash.rs) is for an interactive start; the window opens when it is done.
    let launcher = settings.plugin("launcher") && !hidden && !mini && !measure;

    // A measurement must not read or overwrite the saved window state: it needs the fixed 1280x800.
    let denylist: &[&str] = if measure { &["main", "settings"] } else { &["settings"] };
    let window_state = tauri_plugin_window_state::Builder::new()
        .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
        .with_denylist(denylist)
        .build();

    tauri::Builder::default()
        // Must be the first plugin: a second launch brings the first window back and exits.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second autostart launch (`--hidden`) must not pop the window up.
            if args.iter().any(|a| a == "--mini-player") {
                miniplayer::open(app);
            } else if !args.iter().any(|a| a == "--hidden") {
                show_main(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(window_state)
        .plugin(tauri_plugin_autostart::Builder::new().arg("--hidden").build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(player::Player::new())
        .manage(injections::PageData::default())
        .manage(lyrics::Lyrics::default())
        .manage(audio_facts::Store::new(&profile))
        .manage(settings::Live::new(profile.clone(), args.clone(), settings))
        // Only the settings window gets an answer from it (settings_window.rs).
        .register_uri_scheme_protocol(settings_window::SCHEME, settings_window::protocol)
        .invoke_handler(tauri::generate_handler![
            player::player_state,
            audio_facts::audio_facts,
            settings_window::settings_get,
            settings_window::settings_set,
            settings_window::settings_action,
            settings_window::settings_stats,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let launcher = launcher && splash::show(&handle, profile.clone());
            let window = WindowBuilder::new(app, "main")
                .title("feathertune")
                .inner_size(1280.0, 800.0)
                .min_inner_size(400.0, 300.0)
                .decorations(false)
                .visible(!hidden && !mini && !launcher)
                .build()?;
            let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
            let top = f64::from(titlebar::HEIGHT);
            let site = window.add_child(
                WebviewBuilder::new("site", WebviewUrl::External(url))
                    .data_directory(profile.clone())
                    .additional_browser_args(&args)
                    .initialization_script(script)
                    // A page that (re)loaded lost the data Rust gave its modules; give it again.
                    .on_page_load(|site, payload| {
                        if payload.event() == tauri::webview::PageLoadEvent::Finished {
                            injections::resend(&site);
                            splash::page_loaded(site.app_handle());
                        }
                    })
                    .on_navigation({
                        let handle = handle.clone();
                        move |url| allowed(url) || open_external(&handle, url)
                    })
                    .on_new_window(move |url, _| {
                        open_external(&handle, &url);
                        tauri::webview::NewWindowResponse::Deny
                    }),
                LogicalPosition::new(0.0, top),
                LogicalSize::new(size.width, (size.height - top).max(0.0)),
            )?;
            titlebar::install(app.handle(), &window, &site)?;
            if app.state::<settings::Live>().plugin("taskbar_controls") {
                taskbar::install(app.handle(), &window);
            }
            if hidden || mini {
                set_hidden(app.handle(), true);
            }
            let live = app.state::<settings::Live>();
            if live.plugin("tracker_filter") {
                privacy::install(&site, &profile, measure);
            }
            shortcuts::register(app.handle());
            notify::start(app.handle());
            if live.plugin("discord") {
                let client_id = live.get().discord.client_id.clone();
                discord::start(app.handle(), &client_id);
            }
            let lastfm = if live.plugin("lastfm") {
                let keys = live.get().lastfm.clone();
                lastfm::start(app.handle(), &keys, &profile)
            } else {
                None
            };
            if let Some(l) = &lastfm {
                app.manage(l.clone());
            }
            if live.plugin("album_color") {
                album_color::start(app.handle());
            }
            let page_lyrics = live.plugin("synced_lyrics");
            if page_lyrics || live.plugin("desktop_lyrics") || live.plugin("share_lyrics") {
                lyrics::start(app.handle(), &profile, page_lyrics);
            }
            desktop_lyrics::start(app.handle());
            audio::start(app.handle());
            if live.plugin("listening_stats") {
                stats::start(app.handle(), &profile);
            }
            tray(app.handle(), lastfm)?;
            if let Err(e) = smtc::start(app.handle(), &window) {
                eprintln!("media flyout unavailable: {e}");
            }
            if mini {
                miniplayer::open(app.handle());
            }
            if measure {
                log_player(app.handle(), start);
                measure_probes(site, start);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                set_hidden(window.app_handle(), true);
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run feathertune");
}

/// The WebView2 command line: the single --disable-features plus autoplay.
fn browser_args(settings: &settings::Browser) -> String {
    let features: Vec<&str> = DISABLED_FEATURES.iter().copied().chain(settings.disabled_features()).collect();
    // Animations follow their plugin switches, not Windows' animation effects (which the page would
    // otherwise see as prefers-reduced-motion).
    format!(
        "--disable-features={} --autoplay-policy=no-user-gesture-required --force-prefers-no-reduced-motion",
        features.join(",")
    )
}

fn profile_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into());
    base.join("feathertune")
}

/// In-app navigation: the player domains and the sign-in domains. Everything else opens in the browser.
fn allowed(url: &Url) -> bool {
    if url.scheme() == "about" {
        return true;
    }
    let Some(host) = url.host_str().filter(|_| url.scheme() == "https") else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    if host == "youtube.com" || host.ends_with(".youtube.com") {
        return true;
    }
    let host = ["www.", "accounts.", "consent.", "gds."].iter().find_map(|p| host.strip_prefix(p)).unwrap_or(&host);
    // Sign-in domain and its country variants (.com.br, .co.uk).
    host.strip_prefix("google.").is_some_and(|tld| {
        let parts: Vec<&str> = tld.split('.').collect();
        parts.len() <= 2
            && parts.iter().all(|p| (2..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_lowercase()))
    })
}

/// Opens http(s) links in the default browser. Always returns false (the webview does not navigate).
fn open_external(app: &AppHandle, url: &Url) -> bool {
    if matches!(url.scheme(), "http" | "https")
        && let Err(e) = app.opener().open_url(url.as_str(), None::<&str>)
    {
        eprintln!("could not open {url}: {e}");
    }
    false
}

/// The tray icon. Left click shows or hides the window; right click opens our themed menu
/// (traymenu.rs), or the native one when the `themed_tray_menu` plugin is off.
fn tray(app: &AppHandle, lastfm: Option<lastfm::LastFm>) -> tauri::Result<()> {
    let themed = app.state::<settings::Live>().plugin("themed_tray_menu");
    let mut builder = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("feathertune")
        .show_menu_on_left_click(false);
    if !themed {
        builder = builder.menu(&native_menu(app, lastfm.as_ref())?).on_menu_event(|app, event| {
            tray_command(app, event.id.as_ref());
        });
    }
    builder
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, .. } = event {
                match button {
                    MouseButton::Left => toggle(tray.app_handle()),
                    MouseButton::Right if themed => traymenu::open(tray.app_handle()),
                    _ => {}
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// The Windows menu, used when the themed one is off.
fn native_menu(app: &AppHandle, lastfm: Option<&lastfm::LastFm>) -> tauri::Result<Menu<tauri::Wry>> {
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start with Windows",
        true,
        app.autolaunch().is_enabled().unwrap_or(false),
        None::<&str>,
    )?;
    // Without the keys in config.toml the item only says where they go.
    let lastfm_item = MenuItem::with_id(
        app,
        "lastfm",
        lastfm_label(lastfm.map(lastfm::LastFm::connected)),
        lastfm.is_some(),
        None::<&str>,
    )?;
    if let Some(lastfm) = lastfm {
        let item = lastfm_item.clone();
        lastfm.on_change(move |connected| {
            let _ = item.set_text(lastfm_label(Some(connected)));
        });
    }
    app.manage(AutostartItem(autostart.clone()));
    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "toggle", "Show / hide", true, None::<&str>)?,
            &MenuItem::with_id(app, "play_pause", "Play / pause", true, None::<&str>)?,
            &MenuItem::with_id(app, "next", "Next", true, None::<&str>)?,
            &MenuItem::with_id(app, "mini_player", "Mini player", true, None::<&str>)?,
            &MenuItem::with_id(app, "desktop_lyrics", "Floating lyrics", true, None::<&str>)?,
            &MenuItem::with_id(app, "desktop_lyrics_lock", "Lock / unlock floating lyrics", true, None::<&str>)?,
            &MenuItem::with_id(app, "share_lyrics", "Share lyrics...", true, None::<&str>)?,
            &MenuItem::with_id(app, "reload", "Reload page", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &autostart,
            &lastfm_item,
            &MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )
}

/// What a tray menu item does (both menus use these ids).
pub(crate) fn tray_command(app: &AppHandle, id: &str) {
    match id {
        "toggle" => toggle(app),
        "previous" => player(app, "previous"),
        "play_pause" => player(app, "playPause"),
        "next" => player(app, "next"),
        "mini_player" => miniplayer::toggle(app),
        "desktop_lyrics" => desktop_lyrics::toggle(app),
        "desktop_lyrics_lock" => desktop_lyrics::set_locked(!desktop_lyrics::is_locked()),
        "share_lyrics" => share_lyrics::open(app),
        "reload" => {
            if let Some(site) = app.get_webview("site") {
                let _ = site.eval("location.reload()");
            }
        }
        "settings" => settings_window::open(app),
        "autostart" => {
            let manager = app.autolaunch();
            let on = !manager.is_enabled().unwrap_or(false);
            if let Err(e) = if on { manager.enable() } else { manager.disable() } {
                eprintln!("autostart change failed: {e}");
            }
            sync_autostart_item(app);
        }
        "lastfm" => {
            if let Some(lastfm) = app.try_state::<lastfm::LastFm>() {
                if lastfm.connected() { lastfm.disconnect() } else { lastfm.connect(app) }
            }
        }
        "quit" => {
            stats::flush(app);
            app.exit(0)
        }
        _ => {}
    }
}

/// `None`: the keys are not configured.
pub(crate) fn lastfm_label(connected: Option<bool>) -> &'static str {
    match connected {
        None => "Last.fm: add the keys to config.toml",
        Some(true) => "Disconnect Last.fm",
        Some(false) => "Connect Last.fm",
    }
}

/// The native tray menu's "Start with Windows" item, kept in step with the settings window.
struct AutostartItem(CheckMenuItem<tauri::Wry>);

pub(crate) fn sync_autostart_item(app: &AppHandle) {
    if let Some(item) = app.try_state::<AutostartItem>() {
        let _ = item.0.set_checked(app.autolaunch().is_enabled().unwrap_or(false));
    }
}

fn toggle(app: &AppHandle) {
    let visible = app.get_window("main").is_some_and(|w| w.is_visible().unwrap_or(false));
    set_hidden(app, visible);
}

pub(crate) fn show_main(app: &AppHandle) {
    set_hidden(app, false);
}

/// Runs a command of the `controls` injection (it clicks the player buttons).
pub(crate) fn player(app: &AppHandle, command: &str) {
    if let Some(site) = app.get_webview("site") {
        let _ = site.eval(format!("window.__feathertune?.controls?.{command}()"));
    }
}

/// Hidden: window off, WebView2 not visible and MemoryUsageTargetLevel=Low (page memory goes to
/// the page file and playback continues). Shown: back to visible and Normal, and the mini player
/// closes.
pub(crate) fn set_hidden(app: &AppHandle, hidden: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
    };
    use windows_core::Interface;

    let (Some(window), Some(site)) = (app.get_window("main"), app.get_webview("site")) else { return };
    if hidden {
        // A page left in fullscreen would keep the window fullscreen (titlebar.rs) once it is back.
        let _ = site.eval("document.fullscreenElement && document.exitFullscreen()");
        let _ = window.hide();
    }
    let _ = site.with_webview(move |wv| unsafe {
        let controller = wv.controller();
        let _ = controller.SetIsVisible(!hidden);
        let level = if hidden {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        };
        if let Ok(w19) = controller.CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_19>())
            && let Err(e) = w19.SetMemoryUsageTargetLevel(level)
        {
            eprintln!("SetMemoryUsageTargetLevel failed: {e}");
        }
    });
    if !hidden {
        miniplayer::close(app);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    println!("{} memory {}", if hidden { "hidden," } else { "shown," }, if hidden { "Low" } else { "Normal" });
}

/// Evaluates a script on the main thread and hands its JSON result to `done`.
fn eval_with(webview: &Webview, js: &'static str, done: impl FnOnce(String) + Send + 'static) {
    use webview2_com::ExecuteScriptCompletedHandler;
    use windows_core::HSTRING;

    let _ = webview.with_webview(move |wv| unsafe {
        if let Ok(core) = wv.controller().CoreWebView2() {
            let handler = ExecuteScriptCompletedHandler::create(Box::new(move |_, result| {
                done(result);
                Ok(())
            }));
            let _ = core.ExecuteScript(&HSTRING::from(js), &handler);
        }
    });
}

/// --measure: prints every player event with the time since startup.
fn log_player(app: &AppHandle, start: Instant) {
    let mut rx = app.state::<player::Player>().subscribe();
    std::thread::spawn(move || {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            match rx.blocking_recv() {
                Ok(player::Event::Progress(_)) | Err(RecvError::Lagged(_)) => {}
                Ok(event) => println!("{:>8}ms {event:?}", start.elapsed().as_millis()),
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn measure_probes(site: Webview, start: Instant) {
    std::thread::spawn(move || {
        let found = Arc::new(AtomicBool::new(false));
        while !found.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(60) {
            let found = found.clone();
            eval_with(&site, BAR_PROBE, move |r| {
                if r == "true" && !found.swap(true, Ordering::Relaxed) {
                    println!("player_bar_ms={}", start.elapsed().as_millis());
                }
            });
            std::thread::sleep(Duration::from_millis(100));
        }
        loop {
            std::thread::sleep(Duration::from_secs(10));
            let t = start.elapsed().as_secs();
            eval_with(&site, STATUS_PROBE, move |r| println!("{t:>5}s {r}"));
            eval_with(&site, audio::PROBE, move |r| println!("{t:>5}s {}", r.trim_matches('"')));
        }
    });
}
