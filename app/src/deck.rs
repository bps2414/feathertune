//! The transition voice: for a crossfade (a mix's transition or the global one, mixes.rs), the start
//! of the next track plays in a second, hidden webview (label `deck`) while the main page fades out,
//! so the two overlap. The
//! main page stays the only source of truth (queue, history, scrobbles, lyrics, the media flyout):
//! when the site moves on to the next track, the main page seeks to where the voice is, they cross
//! in 150 ms and the voice is closed. The seconds the voice played are not counted anywhere.
//!
//! - The voice has no capability (it is in no file of `capabilities/`) and only the `deck`
//!   injection: it can call nothing. Rust reads both pages with ExecuteScript (every 250 ms, only
//!   while a voice exists; the main page every 2 s otherwise, while the crossfade is on) and treats
//!   the answers as untrusted: the numbers only time the transition and set the voice's level (the
//!   main page's volume, clamped), and the voice's error text only goes to the log, cleaned (`clean`).
//! - The voice is opened `PREPARE_S` before the fade and must be ready `GO_LEAD_S` before it. Files
//!   are only read for the plan in that window. Each transition picks the mode again: a voice that is
//!   not ready, an ad on either page, a pause, a track change or anything else odd closes the voice
//!   and the main page does the one-player fade.
//! - An ad on the voice closes it at once and turns the voice off until the app restarts (it never
//!   plays one); a voice that stops or errors by itself while playing counts as the account refusing
//!   a second playback, and after two the voice stays off until the app restarts. One that fails
//!   before it plays only falls back. A voice still loading has no player yet: it plays nothing, and
//!   that alone is not an ad. Each transition gets one attempt.
//! - Every transition is logged under `transition` (diag.rs): which mode played and why.

use std::sync::Mutex;
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
/// `overlap` is sent this long before the fade, and `go` once the page accepts it; the voice reaches
/// its in point at the fade, playing silently before it for up to this long (the track's start
/// permitting).
const GO_LEAD_S: f64 = 1.5;
/// How long the main page may take to accept the overlap, before the voice makes a sound.
const ACCEPT_TIMEOUT: Duration = Duration::from_millis(600);
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// How long the main page's state may not fit the plan before the voice gives up (its fade is planned
/// again now and then).
const LEAD_GRACE: Duration = Duration::from_secs(1);
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
/// How the last transition through the voice ended: the log line (for the diagnostics), whether it
/// overlapped, and why not (a key of `locales/`, for the settings).
static LAST: Mutex<Option<(String, bool, &'static str)>> = Mutex::new(None);

fn log(msg: impl std::fmt::Display) {
    crate::diag::log("transition", msg);
}

/// Logs how a transition ended, and keeps it.
fn finished(overlapped: bool, why: &'static str, msg: String) {
    log(&msg);
    *LAST.lock().unwrap() = Some((msg, overlapped, why));
}

/// The voice's error text for the log: a short line of plain text.
fn clean(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric() || " .,:'-".contains(*c)).take(120).collect()
}

/// The voice's state, for the diagnostics.
pub fn status() -> String {
    let voice = if OFF.load(Ordering::Relaxed) { "off until restart" } else { "on" };
    let refusals = REFUSALS.load(Ordering::Relaxed);
    let last = LAST.lock().unwrap().as_ref().map_or_else(|| "none yet".into(), |(msg, ..)| msg.clone());
    format!("voice {voice}, refusals {refusals} of {MAX_REFUSALS}, last: {last}")
}

/// The last transition through the voice, for the settings.
pub fn last() -> Option<serde_json::Value> {
    let off = OFF.load(Ordering::Relaxed);
    LAST.lock()
        .unwrap()
        .as_ref()
        .map(|(_, overlapped, why)| json!({ "overlapped": overlapped, "why": why, "off": off }))
}

/// The main page's state (`audio.deck()`).
#[derive(Deserialize)]
struct Main {
    id: Option<String>,
    t: f64,
    rate: f64,
    paused: bool,
    volume: f64,
    muted: bool,
    ad: bool,
    out: Option<f64>,
    dur: Option<f64>,
    length: Option<f64>,
    armed: bool,
    faded: bool,
    to: Option<String>,
    wall: f64,
    /// The page accepted the `overlap` command.
    overlap: bool,
    waiting: bool,
    done_at: Option<f64>,
    /// How far from the voice the take-over landed (ms), and after how many seeks.
    drift: Option<f64>,
    seeks: u32,
    error: bool,
}

/// The voice's state (`__feathertuneDeck.poll()`).
#[derive(Deserialize)]
struct Voice {
    id: Option<String>,
    ad: bool,
    /// The page has no player yet.
    loading: bool,
    ready: bool,
    started: bool,
    paused: bool,
    t: f64,
    rate: f64,
    wall: f64,
    /// What the page says is wrong (untrusted: only logged, see `clean`).
    error: Option<String>,
    /// Why the browser would not play (the same).
    blocked: Option<String>,
}

/// The transition `mixes::transition` picked (Rust's own data).
#[derive(Deserialize)]
struct Plan {
    mode: String,
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
        // The last track logged as having no transition (once per track, not on every pass).
        let mut no_plan = None;
        let mut last_fade = None;
        // The last transition the voice tried: each one gets a single attempt.
        let mut tried = None;
        loop {
            std::thread::sleep(IDLE);
            if !app.state::<Live>().plugin("crossfade") {
                continue;
            }
            let Some(site) = app.get_webview("site") else { continue };
            let main = read::<Main>(&site, MAIN_POLL);
            if let Some(a) = &main {
                let fade = (a.id.clone(), a.out.map(|o| (o * 10.0).round() as i64), a.armed, a.faded);
                if last_fade.as_ref() != Some(&fade) {
                    log(format_args!(
                        "page: at {:.1} of {:?} s, fade at {:?} for {:?} s into {:?}, armed: {}, faded: {}",
                        a.t, a.length, a.out, a.dur, a.to, a.armed, a.faded
                    ));
                    last_fade = Some(fade);
                }
            }
            if OFF.load(Ordering::Relaxed) {
                continue;
            }
            let Some(a) = main else { continue };
            if !until_fade(&a).is_some_and(|left| (MIN_PREPARE_S..=PREPARE_S).contains(&left)) {
                continue;
            }
            let planned = next_plan(&app).and_then(|(plan, bpm)| lead(&a, &plan).map(|left| (plan, bpm, left)));
            let Some((plan, bpm, left)) = planned else {
                if a.id != no_plan {
                    no_plan = a.id.clone();
                    let (list, next) = app.state::<crate::mixes::Mixes>().now();
                    log(format_args!(
                        "one player: no transition (playlist: {}, next track: {}, the page's next: {:?})",
                        list.is_some(),
                        next.is_some(),
                        a.to
                    ));
                }
                continue;
            };
            let pair = Some((plan.from.clone(), plan.to.clone()));
            if tried == pair {
                continue;
            }
            tried = pair;
            log(format_args!("voice opens {left:.1} s before the fade ({} into {})", plan.mode, plan.to));
            match mix(&app, &site, &plan, bpm) {
                Outcome::Done => finished(true, "", "overlap: the main page took over".into()),
                Outcome::Fallback(why) => finished(false, why, format!("one player this time ({why})")),
                Outcome::Refused(why) => {
                    let n = REFUSALS.fetch_add(1, Ordering::Relaxed) + 1;
                    if n >= MAX_REFUSALS {
                        OFF.store(true, Ordering::Relaxed);
                    }
                    finished(false, why, format!("refused ({why}), {n} of {MAX_REFUSALS}"));
                }
                Outcome::AdOnVoice => {
                    OFF.store(true, Ordering::Relaxed);
                    finished(false, "an ad on the voice", "an ad on the voice, off until restart".into());
                }
            }
        }
    });
}

/// The transition out of the current track, with both tracks' tempos.
fn next_plan(app: &AppHandle) -> Option<(Plan, Tempos)> {
    let current = app.state::<Player>().current().and_then(|s| s.track.id)?;
    let plan: Plan = serde_json::from_value(crate::mixes::transition(app, &current)?).ok()?;
    let store = app.state::<crate::audio_facts::Store>();
    let bpm = (store.load(&plan.from).and_then(|f| f.bpm), store.load(&plan.to).and_then(|f| f.bpm));
    Some((plan, bpm))
}

/// Seconds (wall time) from the main page's state until its fade.
fn until_fade(a: &Main) -> Option<f64> {
    (0.25..=4.0).contains(&a.rate).then(|| a.out.map(|out| (out - a.t) / a.rate))?
}

/// `until_fade`, if the main page plays the plan's track and nothing is in the way.
fn lead(a: &Main, plan: &Plan) -> Option<f64> {
    let ok = a.id.as_deref() == Some(&plan.from) && a.to.as_deref() == Some(&plan.to) && !a.paused && !a.ad && !a.error;
    ok.then(|| until_fade(a))?
}

/// The main page's volume, for the voice.
fn level(a: &Main) -> serde_json::Value {
    json!({ "volume": a.volume.clamp(0.0, 1.0), "muted": a.muted })
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
    let opened = Instant::now();
    let mut seen_ready = false;
    let mut lost: Option<Instant> = None;
    // Waiting for the fade, with the voice loading and seeking to its in point.
    let a = loop {
        std::thread::sleep(POLL);
        let Some(a) = read::<Main>(site, MAIN_POLL) else { return Outcome::Fallback("no answer from the page") };
        let v = read::<Voice>(voice, &poll);
        if let Some(v) = &v {
            if v.ad {
                return Outcome::AdOnVoice;
            }
            // Nothing plays yet, so this is not the account refusing a second playback.
            if let Some(error) = &v.error {
                log(format_args!("voice error before it played: {}", clean(error)));
                return Outcome::Fallback("the voice shows an error");
            }
        }
        let Some(left) = lead(&a, plan) else {
            if lost.get_or_insert_with(Instant::now).elapsed() < LEAD_GRACE {
                continue;
            }
            log(format_args!(
                "main page: on {:?}, next {:?}, fade at {:?}, paused: {}, ad: {}",
                a.id, a.to, a.out, a.paused, a.ad
            ));
            return Outcome::Fallback("the main page changed");
        };
        lost = None;
        let ready = v.as_ref().is_some_and(|v| v.ready && v.id.as_deref() == Some(&plan.to));
        if ready && !seen_ready {
            seen_ready = true;
            log(format_args!("voice ready after {:.1} s, {left:.1} s before the fade", opened.elapsed().as_secs_f64()));
        }
        if left > GO_LEAD_S {
            continue;
        }
        if !ready {
            return Outcome::Fallback(match &v {
                None => "the voice did not answer",
                Some(v) if v.loading => "the voice was still loading",
                Some(_) => "the voice was not ready in time",
            });
        }
        if left < 0.2 {
            return Outcome::Fallback("the fade was too close to start the voice");
        }
        break a;
    };
    let at_ms = a.wall + lead(&a, plan).unwrap_or(0.0) * 1000.0;
    let beat_ms = bpm.0.map_or(500.0, |b| 60_000.0 / b);
    let rate = match bpm {
        (Some(a), Some(b)) if plan.effects.tempo && ((a / b) - 1.0).abs() <= MAX_RATE_DELTA => a / b,
        _ => 1.0,
    };
    let mut sent_level = level(&a);
    let go = json!({
        "play_ms": at_ms - (plan.in_ms - in_s * 1000.0) / rate, "at_ms": at_ms, "dur_ms": plan.duration_ms,
        "curve": plan.curve, "rate": rate, "bass_swap": plan.effects.bass_swap, "beat_ms": beat_ms,
        "level": sent_level,
    });
    command(
        app,
        "overlap",
        json!({ "to": plan.to, "at_ms": at_ms, "dur_ms": plan.duration_ms,
        "bass_swap": plan.effects.bass_swap, "beat_ms": beat_ms }),
    );
    // No sound from the voice unless the page will hold the next track for it.
    if !accepted(site) {
        return Outcome::Fallback("the page did not accept the overlap");
    }
    let _ = voice.eval(format!("window.__feathertuneDeck?.go({go})"));

    // The overlap, then the take-over.
    let started = Instant::now();
    let limit = Duration::from_secs_f64(plan.duration_ms / 1000.0) + HANDOFF_TIMEOUT;
    let mut handed = false;
    let mut checked = false;
    loop {
        std::thread::sleep(POLL);
        if started.elapsed() > limit {
            return Outcome::Fallback("the main page did not take over in time");
        }
        let (Some(a), Some(v)) = (read::<Main>(site, MAIN_POLL), read::<Voice>(voice, &poll)) else {
            return Outcome::Fallback("no answer from a page");
        };
        if !checked {
            checked = true;
            log(format_args!("voice started: {}, page accepted the overlap: {}", v.started, a.overlap));
        }
        if a.ad {
            return Outcome::Fallback("an ad on the main page");
        }
        if v.ad {
            return Outcome::AdOnVoice;
        }
        // The page went back to one player by itself: the voice must not sound over it.
        if !a.overlap {
            return Outcome::Fallback("the page dropped the overlap");
        }
        if let Some(why) = &v.blocked {
            log(format_args!("the voice could not play: {}", clean(why)));
            return Outcome::Fallback("the voice could not play");
        }
        if let Some(error) = &v.error {
            log(format_args!("voice error: {}", clean(error)));
        }
        if !v.started {
            return Outcome::Fallback("the voice did not start");
        }
        if v.error.is_some() || v.paused {
            return Outcome::Refused("the voice stopped by itself");
        }
        if a.error || a.paused {
            return Outcome::Fallback("the main page paused");
        }
        let on = a.id.as_deref().filter(|id| valid_id(id));
        if on != Some(plan.from.as_str()) && on != Some(plan.to.as_str()) {
            return Outcome::Fallback("another track started");
        }
        let now_level = level(&a);
        if now_level != sent_level {
            let _ = voice.eval(format!("window.__feathertuneDeck?.volume({now_level})"));
            sent_level = now_level;
        }
        if let Some(done) = a.done_at {
            log(format_args!("take-over {:?} ms from the voice, after {} seeks", a.drift, a.seeks));
            let _ = voice
                .eval(format!("window.__feathertuneDeck?.fadeOut({})", json!({ "at_ms": done, "ms": HANDOFF_MS })));
            std::thread::sleep(Duration::from_millis(1000));
            return Outcome::Done;
        }
        if !handed && a.waiting && on == Some(plan.to.as_str()) && (0.25..=4.0).contains(&v.rate) {
            command(app, "handoff", json!({ "t": v.t, "wall_ms": v.wall, "rate": v.rate }));
            handed = true;
            log(format_args!("handoff at {:.1} s of the next track", v.t));
        }
    }
}

/// Whether the main page took the `overlap` command, within `ACCEPT_TIMEOUT`.
fn accepted(site: &Webview) -> bool {
    let asked = Instant::now();
    while asked.elapsed() < ACCEPT_TIMEOUT {
        if read::<Main>(site, MAIN_POLL).is_some_and(|a| a.overlap) {
            return true;
        }
        std::thread::sleep(ACCEPT_POLL);
    }
    false
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
        .map_err(|e| log(format_args!("the voice did not open: {e}")))
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
