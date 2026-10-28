//! Diagnostics: the last `MAX_LINES` log lines in memory (the release exe has no console, so
//! `eprintln!` alone is lost), and a text report with them that the settings window saves to
//! Downloads ("Export diagnostics"). Nothing is written to disk or sent anywhere until that is
//! pressed; the person sends the file on their own.
//!
//! The report has the app and WebView2 versions, the plugin switches, the lyrics settings, the
//! current track and lyrics, and the log. It names the tracks played since the app started (the
//! lyrics lookups log them), never tokens, keys, account names or paths.

use std::collections::VecDeque;
use std::fmt::{Display, Write as _};
use std::sync::Mutex;
use std::time::Instant;

use tauri::{AppHandle, Manager};

use crate::settings::Live;

const MAX_LINES: usize = 600;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Marks the start of the clock the log lines are stamped with (seconds since start).
pub fn start() {
    START.get_or_init(Instant::now);
}

/// Logs `msg` under `area` (also to stderr, for debug builds).
pub fn log(area: &str, msg: impl Display) {
    let at = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    let line = format!("{at:9.3}s {area}: {msg}");
    eprintln!("{line}");
    let mut log = LOG.lock().unwrap();
    if log.len() == MAX_LINES {
        log.pop_front();
    }
    log.push_back(line);
}

/// The whole report, as text.
pub fn report(app: &AppHandle) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "feathertune {} diagnostics", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "WebView2: {}", tauri::webview_version().unwrap_or_else(|e| format!("unknown ({e})")));
    let _ = writeln!(out, "Uptime: {:.0} s", START.get_or_init(Instant::now).elapsed().as_secs_f64());
    let _ = writeln!(out, "Game mode: {}", if crate::game_mode::is_on() { "on" } else { "off" });

    let live = app.state::<Live>();
    let settings = live.get();
    let off: Vec<&str> = crate::plugins::PLUGINS.iter().filter(|p| !settings.plugin(p.id)).map(|p| p.id).collect();
    let _ = writeln!(out, "\n[plugins off] {}", if off.is_empty() { "none".to_owned() } else { off.join(", ") });
    let running: Vec<String> = app
        .state::<crate::extensions::Extensions>()
        .running()
        .iter()
        .map(|e| format!("{} {}", e.id, e.version))
        .collect();
    let _ = writeln!(
        out,
        "[extensions running] {}",
        if running.is_empty() { "none".to_owned() } else { running.join(", ") }
    );
    let l = &settings.lyrics;
    let _ = writeln!(
        out,
        "[lyrics] providers = {:?}, estimate_words = {}, offset_ms = {}, translate_to = {:?} (target: {:?})",
        l.providers,
        l.estimate_words,
        l.offset_ms,
        l.translate_to,
        crate::lyrics::translation_target(&settings)
    );
    drop(settings);

    let _ = writeln!(out, "\n[now]");
    match app.state::<crate::player::Player>().current() {
        Some(s) => {
            let t = &s.track;
            let _ = writeln!(
                out,
                "track: {:?} by {:?}, album {:?}, id {:?}, duration {:.1} s, at {:.1} s, {}",
                t.title,
                t.artist,
                t.album,
                t.id,
                t.duration,
                s.position_now(),
                if s.paused { "paused" } else { "playing" }
            );
        }
        None => out.push_str("track: none\n"),
    }
    match app.try_state::<crate::lyrics::Lyrics>().and_then(|l| l.current()) {
        Some(s) => {
            let _ = writeln!(
                out,
                "lyrics: {} lines from {}, words: {}, estimated: {}, translation: {:?}",
                s.lines.len(),
                s.source,
                s.lines.iter().any(|l| !l.words.is_empty()),
                s.estimated,
                s.translated
            );
        }
        None => out.push_str("lyrics: none\n"),
    }

    let _ = writeln!(out, "\n[log] (seconds since start)");
    for line in LOG.lock().unwrap().iter() {
        out.push_str(line);
        out.push('\n');
    }
    out
}
