// feathertune: the official web music player in a lean WebView2 window, with a tray icon.
// Usage: feathertune [--measure] [start url on the player domain]
// --measure prints the time until the player bar shows up and, every 10 s, the <video> time and
// login state, so measure.ps1 can record them. The page can call a single Tauri command,
// `player_state` (see player.rs and capabilities/), which only reports data.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod injections;
mod player;
mod smtc;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_opener::OpenerExt;

const START_URL: &str = "https://music.youtube.com";
// wry's defaults (dropped once we pass our own args), Chromium's media keys and flyout entry (ours
// is in smtc.rs) and autoplay. Keep every feature in this single --disable-features: Chromium
// only honors the last one.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,HardwareMediaKeyHandling --autoplay-policy=no-user-gesture-required";

const BAR_PROBE: &str = "(() => { const b = document.querySelector('ytmusic-player-bar'); \
    return !!b && b.getBoundingClientRect().height > 0 && !!b.querySelector('.title')?.textContent.trim() })()";
const STATUS_PROBE: &str = "(() => { const v = document.querySelector('video'); \
    return { time: v ? Math.round(v.currentTime) : null, paused: v ? v.paused : null, \
    loggedIn: window.ytcfg?.get?.('LOGGED_IN') ?? null } })()";

fn main() {
    let start = Instant::now();
    let mut measure = false;
    let mut url: Url = START_URL.parse().unwrap();
    for arg in std::env::args().skip(1) {
        if arg == "--measure" {
            measure = true;
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

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(player::Player::new())
        .invoke_handler(tauri::generate_handler![player::player_state])
        .setup(move |app| {
            let handle = app.handle().clone();
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("feathertune")
                .inner_size(1280.0, 800.0)
                .data_directory(profile)
                .additional_browser_args(BROWSER_ARGS)
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
            tray(app.handle())?;
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

fn tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "toggle", "Show / hide", true, None::<&str>)?,
            &MenuItem::with_id(app, "play_pause", "Play / pause", true, None::<&str>)?,
            &MenuItem::with_id(app, "next", "Next", true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("feathertune")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => toggle(app),
            "play_pause" => player(app, "playPause"),
            "next" => player(app, "next"),
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
