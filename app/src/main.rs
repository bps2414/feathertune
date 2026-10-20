// feathertune: the official web music player in a lean WebView2 window, with a tray icon.
// Usage: feathertune [--measure] [--hidden] [start url on the player domain]
// --hidden starts in the tray (the autostart entry passes it).
// --measure prints the time until the player bar shows up and, every 10 s, the <video> time and
// login state, so measure.ps1 can record them. The page can call a single Tauri command,
// `player_state` (see player.rs and capabilities/), which only reports data.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discord;
mod injections;
mod lastfm;
mod notify;
mod player;
mod privacy;
mod settings;
mod shortcuts;
mod smtc;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

const START_URL: &str = "https://music.youtube.com";
// wry's defaults (dropped once we pass our own args), Chromium's media keys and flyout entry (ours
// is in smtc.rs). Keep every feature in this single --disable-features: Chromium only honors the
// last one. Features that the Phase 4 experiments kept go here; the ones still being measured come
// from `[browser] disable_features` in config.toml (see `browser_args`).
const DISABLED_FEATURES: &[&str] = &["msWebOOUI", "msPdfOOUI", "msSmartScreenProtection", "HardwareMediaKeyHandling"];

const BAR_PROBE: &str = "(() => { const b = document.querySelector('ytmusic-player-bar'); \
    return !!b && b.getBoundingClientRect().height > 0 && !!b.querySelector('.title')?.textContent.trim() })()";
const STATUS_PROBE: &str = "(() => { const v = document.querySelector('video'); \
    return { time: v ? Math.round(v.currentTime) : null, paused: v ? v.paused : null, \
    loggedIn: window.ytcfg?.get?.('LOGGED_IN') ?? null } })()";

fn main() {
    let start = Instant::now();
    let mut measure = false;
    let mut hidden = false;
    let mut url: Url = START_URL.parse().unwrap();
    for arg in std::env::args().skip(1) {
        if arg == "--measure" {
            measure = true;
        } else if arg == "--hidden" {
            hidden = true;
        } else if let Ok(u) = arg.parse::<Url>()
            && u.scheme() == "https"
            && u.host_str() == Some("music.youtube.com")
        {
            url = u;
        }
    }
    smtc::set_app_id();
    let profile = profile_dir();
    let config = injections::Config::load(&profile);
    let settings = settings::Settings::load(&profile);

    // A measurement must not read or overwrite the saved window state: it needs the fixed 1280x800.
    let denylist: &[&str] = if measure { &["main"] } else { &[] };
    let window_state = tauri_plugin_window_state::Builder::new()
        .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
        .with_denylist(denylist)
        .build();

    tauri::Builder::default()
        // Must be the first plugin: a second launch brings the first window back and exits.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second autostart launch (`--hidden`) must not pop the window up.
            if !args.iter().any(|a| a == "--hidden")
                && let Some(w) = app.get_webview_window("main")
            {
                set_hidden(&w, false);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(window_state)
        .plugin(tauri_plugin_autostart::Builder::new().arg("--hidden").build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(player::Player::new())
        .invoke_handler(tauri::generate_handler![player::player_state])
        .setup(move |app| {
            let handle = app.handle().clone();
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("feathertune")
                .inner_size(1280.0, 800.0)
                .data_directory(profile.clone())
                .visible(!hidden)
                .additional_browser_args(&browser_args(&settings.browser))
                .initialization_script(injections::script(&config))
                .on_navigation({
                    let handle = handle.clone();
                    move |url| allowed(url) || open_external(&handle, url)
                })
                .on_new_window(move |url, _| {
                    open_external(&handle, &url);
                    tauri::webview::NewWindowResponse::Deny
                })
                .build()?;
            if hidden {
                set_hidden(&window, true);
            }
            if settings.privacy.tracker_filter {
                privacy::install(&window, &profile, measure);
            }
            shortcuts::register(app.handle(), &settings.shortcuts);
            if settings.notifications.track_change {
                notify::start(app.handle());
            }
            discord::start(app.handle(), &settings.discord.client_id);
            let lastfm = lastfm::start(app.handle(), &settings.lastfm, &profile);
            tray(app.handle(), lastfm)?;
            if let Err(e) = smtc::start(app.handle(), &window) {
                eprintln!("media flyout unavailable: {e}");
            }
            if measure {
                log_player(app.handle(), start);
                measure_probes(window, start);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Some(w) = window.get_webview_window("main") {
                    set_hidden(&w, true);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run feathertune");
}

/// The WebView2 command line: the single --disable-features plus autoplay.
fn browser_args(settings: &settings::Browser) -> String {
    let features: Vec<&str> = DISABLED_FEATURES.iter().copied().chain(settings.disabled_features()).collect();
    format!("--disable-features={} --autoplay-policy=no-user-gesture-required", features.join(","))
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

fn tray(app: &AppHandle, lastfm: Option<lastfm::LastFm>) -> tauri::Result<()> {
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
        lastfm_label(lastfm.as_ref().map(lastfm::LastFm::connected)),
        lastfm.is_some(),
        None::<&str>,
    )?;
    if let Some(lastfm) = &lastfm {
        let item = lastfm_item.clone();
        lastfm.on_change(move |connected| {
            let _ = item.set_text(lastfm_label(Some(connected)));
        });
    }
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "toggle", "Show / hide", true, None::<&str>)?,
            &MenuItem::with_id(app, "play_pause", "Play / pause", true, None::<&str>)?,
            &MenuItem::with_id(app, "next", "Next", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &autostart,
            &lastfm_item,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("feathertune")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "toggle" => toggle(app),
            "play_pause" => player(app, "playPause"),
            "next" => player(app, "next"),
            "autostart" => {
                // The item has already flipped itself when the event arrives.
                let on = autostart.is_checked().unwrap_or(false);
                let manager = app.autolaunch();
                if let Err(e) = if on { manager.enable() } else { manager.disable() } {
                    eprintln!("autostart change failed: {e}");
                    let _ = autostart.set_checked(!on);
                }
            }
            "lastfm" => {
                if let Some(lastfm) = &lastfm {
                    if lastfm.connected() { lastfm.disconnect() } else { lastfm.connect(app) }
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// `None`: the keys are not configured.
fn lastfm_label(connected: Option<bool>) -> &'static str {
    match connected {
        None => "Last.fm: add the keys to config.toml",
        Some(true) => "Disconnect Last.fm",
        Some(false) => "Connect Last.fm",
    }
}

fn toggle(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        set_hidden(&w, w.is_visible().unwrap_or(false));
    }
}

/// Runs a command of the `controls` injection (it clicks the player buttons).
pub(crate) fn player(app: &AppHandle, command: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(format!("window.__feathertune?.controls?.{command}()"));
    }
}

/// Hidden: window off, WebView2 not visible and MemoryUsageTargetLevel=Low (page memory goes to
/// the page file and playback continues). Shown: back to visible and Normal.
fn set_hidden(window: &WebviewWindow, hidden: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
    };
    use windows_core::Interface;

    if hidden {
        let _ = window.hide();
    }
    let _ = window.with_webview(move |wv| unsafe {
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
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    println!("{} memory {}", if hidden { "hidden," } else { "shown," }, if hidden { "Low" } else { "Normal" });
}

/// Evaluates a script on the main thread and hands its JSON result to `done`.
fn eval_with(window: &WebviewWindow, js: &'static str, done: impl FnOnce(String) + Send + 'static) {
    use webview2_com::ExecuteScriptCompletedHandler;
    use windows_core::HSTRING;

    let _ = window.with_webview(move |wv| unsafe {
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

fn measure_probes(window: WebviewWindow, start: Instant) {
    std::thread::spawn(move || {
        let found = Arc::new(AtomicBool::new(false));
        while !found.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(60) {
            let found = found.clone();
            eval_with(&window, BAR_PROBE, move |r| {
                if r == "true" && !found.swap(true, Ordering::Relaxed) {
                    println!("player_bar_ms={}", start.elapsed().as_millis());
                }
            });
            std::thread::sleep(Duration::from_millis(100));
        }
        loop {
            std::thread::sleep(Duration::from_secs(10));
            let t = start.elapsed().as_secs();
            eval_with(&window, STATUS_PROBE, move |r| println!("{t:>5}s {r}"));
        }
    });
}
