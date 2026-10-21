//! The transition voice: for a mix's transition (mixes.rs), the start of the next track plays in a
//! second, hidden webview (label `deck`) while the main page fades out, so the two overlap. The
//! main page stays the only source of truth (queue, history, scrobbles, lyrics, the media flyout):
//! when the site moves on to the next track, the main page seeks to where the voice is, they cross
//! in 150 ms and the voice is closed. The seconds the voice played are not counted anywhere.
//!
//! - The voice has no capability (it is in no file of `capabilities/`) and only the `deck`
//!   injection: it can call nothing. Rust reads both pages with ExecuteScript (every 250 ms, only
//!   while a voice exists; every 2 s otherwise, only while a mix's transition is next) and treats
//!   the answers as untrusted numbers: they only time the transition and are never passed on.
//! - The voice is opened `PREPARE_S` before the fade and must be ready `GO_LEAD_S` before it. Each
//!   transition picks the mode again: a voice that is not ready, an ad on either page, a pause, a
//!   track change or anything else odd closes the voice and the main page does the one-player fade.
//! - An ad on the voice closes it at once and turns the voice off until the app restarts (it never
//!   plays one); a voice that stops or errors by itself counts as the account refusing a second
//!   playback, and after two the voice stays off until the app restarts.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use tauri::webview::{NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl};

use crate::mixes::Effects;
use crate::player::{Player, valid_id};
use crate::settings::Live;

const LABEL: &str = "deck";
const SCRIPT: &str = include_str!("../injections/deck/page.js");
const MAIN_POLL: &str = "window.__feathertune?.audio?.deck?.() ?? null";
/// The voice opens this long before the fade (main page time), and not with less than the minimum.
const PREPARE_S: f64 = 40.0;
const MIN_PREPARE_S: f64 = 10.0;
/// `go` is sent this long before the fade; the voice waits this much before its in point.
const GO_LEAD_S: f64 = 1.0;
const POLL: Duration = Duration::from_millis(250);
const IDLE: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// After the fade, how long the main page may take to take over.
const HANDOFF_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_REFUSALS: u32 = 2;
/// The tempo effect: only within ±6 %.
const MAX_RATE_DELTA: f64 = 0.06;
const HANDOFF_MS: f64 = 150.0;

static REFUSALS: AtomicU32 = AtomicU32::new(0);
static OFF: AtomicBool = AtomicBool::new(false);

/// The main page's state (`audio.deck()`).
#[derive(Deserialize)]
struct Main {
    id: Option<String>,
    t: f64,
    rate: f64,
    paused: bool,
    ad: bool,
    out: Option<f64>,
    to: Option<String>,
    wall: f64,
    waiting: bool,
    done_at: Option<f64>,
    error: bool,
}

/// The voice's state (`__feathertuneDeck.poll()`).
#[derive(Deserialize)]
struct Voice {
    id: Option<String>,
    ad: bool,
    ready: bool,
    started: bool,
    paused: bool,
    t: f64,
    rate: f64,
    wall: f64,
    error: bool,
}

/// The transition `mixes::plan` picked (Rust's own data).
#[derive(Deserialize)]
struct Plan {
    from: String,
    to: String,
    in_ms: f64,
    duration_ms: f64,
    curve: String,
    effects: Effects,
}

enum Outcome {
    Done,
    /// Back to one player for this transition.
    Fallback(&'static str),
    Refused(&'static str),
    AdOnVoice,
}

/// The outgoing and incoming tracks' BPM, when measured.
type Tempos = (Option<f64>, Option<f64>);

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(IDLE);
            if OFF.load(Ordering::Relaxed) || !app.state::<Live>().plugin("crossfade") {
                continue;
            }
            let (Some(site), Some((plan, bpm))) = (app.get_webview("site"), next_plan(&app)) else { continue };
            let Some(a) = read::<Main>(&site, MAIN_POLL) else { continue };
            let Some(left) = lead(&a, &plan) else { continue };
            if !(MIN_PREPARE_S..=PREPARE_S).contains(&left) {
                continue;
            }
            match mix(&app, &site, &plan, bpm) {
                Outcome::Done => {}
                Outcome::Fallback(why) => eprintln!("transition voice: one player this time ({why})"),
                Outcome::Refused(why) => {
                    let n = REFUSALS.fetch_add(1, Ordering::Relaxed) + 1;
                    eprintln!("transition voice: refused ({why}), {n} of {MAX_REFUSALS}");
                    if n >= MAX_REFUSALS {
                        OFF.store(true, Ordering::Relaxed);
                    }
                }
                Outcome::AdOnVoice => {
                    eprintln!("transition voice: an ad on the voice, off until restart");
                    OFF.store(true, Ordering::Relaxed);
                }
            }
        }
    });
}

/// The mix's transition out of the current track, with both tracks' tempos.
fn next_plan(app: &AppHandle) -> Option<(Plan, Tempos)> {
    let current = app.state::<Player>().current().and_then(|s| s.track.id)?;
    let data = crate::mixes::page_data(app, Some(&current));
    let plan: Plan = serde_json::from_value(data["transition"].clone()).ok()?;
    let store = app.state::<crate::audio_facts::Store>();
    let bpm = (store.load(&plan.from).and_then(|f| f.bpm), store.load(&plan.to).and_then(|f| f.bpm));
    Some((plan, bpm))
}

/// Seconds (wall time) from the main page's state until its fade, if it plays the plan's track.
fn lead(a: &Main, plan: &Plan) -> Option<f64> {
    let ok = a.id.as_deref() == Some(&plan.from)
        && a.to.as_deref() == Some(&plan.to)
        && !a.paused
        && !a.ad
        && !a.error
        && (0.25..=4.0).contains(&a.rate);
    ok.then(|| a.out.map(|out| (out - a.t) / a.rate))?
}

/// One transition through the voice.
fn mix(app: &AppHandle, site: &Webview, plan: &Plan, bpm: Tempos) -> Outcome {
    let Some(voice) = open(app, &plan.to) else { return Outcome::Fallback("the voice did not open") };
    let outcome = run(app, site, &voice, plan, bpm);
    if !matches!(outcome, Outcome::Done) {
        let _ = voice.eval("window.__feathertuneDeck?.stop()");
        command(app, "overlap_off", json!(true));
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = voice.close();
    outcome
}

fn run(app: &AppHandle, site: &Webview, voice: &Webview, plan: &Plan, bpm: Tempos) -> Outcome {
    let in_s = (plan.in_ms / 1000.0 - GO_LEAD_S).max(0.0);
    let poll = format!("window.__feathertuneDeck?.poll({in_s}) ?? null");
    // Waiting for the fade, with the voice loading and seeking to its in point.
    let a = loop {
        std::thread::sleep(POLL);
        let Some(a) = read::<Main>(site, MAIN_POLL) else { return Outcome::Fallback("no answer from the page") };
        let v = read::<Voice>(voice, &poll);
        if let Some(v) = &v {
            if v.ad {
                return Outcome::AdOnVoice;
            }
            if v.error {
                return Outcome::Refused("the voice shows an error");
            }
        }
        let Some(left) = lead(&a, plan) else { return Outcome::Fallback("the main page changed") };
        if left > GO_LEAD_S {
            continue;
        }
        let ready = v.is_some_and(|v| v.ready && v.id.as_deref() == Some(&plan.to));
        if !ready || left < 0.2 {
            return Outcome::Fallback("the voice was not ready in time");
        }
        break a;
    };
    let at_ms = a.wall + lead(&a, plan).unwrap_or(0.0) * 1000.0;
    let beat_ms = bpm.0.map_or(500.0, |b| 60_000.0 / b);
    let rate = match bpm {
        (Some(a), Some(b)) if plan.effects.tempo && ((a / b) - 1.0).abs() <= MAX_RATE_DELTA => a / b,
        _ => 1.0,
    };
    let go = json!({
        "at_ms": at_ms, "dur_ms": plan.duration_ms, "curve": plan.curve,
        "rate": rate, "bass_swap": plan.effects.bass_swap, "beat_ms": beat_ms,
    });
    command(
        app,
        "overlap",
        json!({ "to": plan.to, "at_ms": at_ms, "dur_ms": plan.duration_ms,
        "bass_swap": plan.effects.bass_swap, "beat_ms": beat_ms }),
    );
    let _ = voice.eval(format!("window.__feathertuneDeck?.go({go})"));

    // The overlap, then the take-over.
    let started = Instant::now();
    let limit = Duration::from_secs_f64(plan.duration_ms / 1000.0) + HANDOFF_TIMEOUT;
    let mut handed = false;
    loop {
        std::thread::sleep(POLL);
        if started.elapsed() > limit {
            return Outcome::Fallback("the main page did not take over in time");
        }
        let (Some(a), Some(v)) = (read::<Main>(site, MAIN_POLL), read::<Voice>(voice, &poll)) else {
            return Outcome::Fallback("no answer from a page");
        };
        if a.ad {
            return Outcome::Fallback("an ad on the main page");
        }
        if v.ad {
            return Outcome::AdOnVoice;
        }
        if v.error || (v.started && v.paused) {
            return Outcome::Refused("the voice stopped by itself");
        }
        if a.error || a.paused {
            return Outcome::Fallback("the main page paused");
        }
        let on = a.id.as_deref().filter(|id| valid_id(id));
        if on != Some(plan.from.as_str()) && on != Some(plan.to.as_str()) {
            return Outcome::Fallback("another track started");
        }
        if let Some(done) = a.done_at {
            let _ = voice
                .eval(format!("window.__feathertuneDeck?.fadeOut({})", json!({ "at_ms": done, "ms": HANDOFF_MS })));
            std::thread::sleep(Duration::from_millis(1000));
            return Outcome::Done;
        }
        if !handed && a.waiting && on == Some(plan.to.as_str()) && (0.25..=4.0).contains(&v.rate) {
            command(app, "handoff", json!({ "t": v.t, "wall_ms": v.wall, "rate": v.rate }));
            handed = true;
        }
    }
}

/// A command for the main page's audio module (run once there, see mixes.rs).
fn command(app: &AppHandle, name: &str, data: serde_json::Value) {
    if let Some(mixes) = app.try_state::<crate::mixes::Mixes>() {
        mixes.command(name, data);
        crate::audio::push(app);
    }
}

/// The hidden voice webview, with the same profile and browser arguments as the site (WebView2
/// needs both to share one browser process) and only the `deck` script.
fn open(app: &AppHandle, to: &str) -> Option<Webview> {
    if let Some(old) = app.get_webview(LABEL) {
        let _ = old.close();
    }
    let window = app.get_window("main")?;
    let live = app.state::<Live>();
    let url: Url = format!("https://music.youtube.com/watch?v={to}").parse().ok()?;
    let script = format!("(() => {{\nif (location.origin !== 'https://music.youtube.com') return;\n{SCRIPT}\n}})();\n");
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(url))
        .data_directory(live.profile.clone())
        .additional_browser_args(&live.browser_args)
        .initialization_script(script)
        .focused(false)
        .on_navigation(|url| url.scheme() == "https" && url.host_str() == Some("music.youtube.com"))
        .on_new_window(|_, _| NewWindowResponse::Deny);
    let voice = window
        .add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(1.0, 1.0))
        .map_err(|e| eprintln!("transition voice: {e}"))
        .ok()?;
    let _ = voice.with_webview(|wv| unsafe {
        let _ = wv.controller().SetIsVisible(false);
    });
    if live.plugin("tracker_filter") {
        crate::privacy::install(&voice, &live.profile, false);
    }
    Some(voice)
}

/// Runs `js` in `webview` and parses its result; `None` on no answer, `null` or a bad shape.
fn read<T: DeserializeOwned>(webview: &Webview, js: &str) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    crate::eval_with(webview, js, move |result| {
        let _ = tx.send(result);
    });
    serde_json::from_str(&rx.recv_timeout(READ_TIMEOUT).ok()?).ok()
}
