//! The transition voice: for an overlapping crossfade (a mix's transition or the global one), the END
//! of the current track plays in a second, hidden webview (label `deck`) while the main page moves on to
//! the next track at the start of the fade, so the two overlap. The main page stays the only source of
//! truth (queue, history, scrobbles, lyrics, the media flyout) and changes track with the sound; the
//! voice only plays the last seconds of the track the main page has just played, and is closed when that
//! track ends. The seconds the voice played are not counted anywhere.
//!
//! - The voice has no capability (it is in no file of `capabilities/`) and only the `deck` injection:
//!   it can call nothing. Rust reads both pages with ExecuteScript and treats the answers as untrusted:
//!   the numbers (clamped, `Main::clamped`) only time the transition, keep the voice in step and set its
//!   level, and the voice's error text only goes to the log, cleaned (`clean`).
//! - Clocks: each page has its own wall clock (performance.timeOrigin + now(), anchored by its own
//!   process). Rust measures both against its own (the midpoint of a read, from the fastest round trip of
//!   the last few, `Offset`) and moves the main page's times onto the voice's (`Clocks::skew`).
//! - Phases: the voice opens `OPEN_S` before the fade and waits `PRE_ROLL_S` before it (`poll`). `GO_S`
//!   before the fade (if ready by `READY_BY_S`) it starts, silent, where the main page will be, and Rust
//!   keeps it in step every 100 ms (`sync`: one seek when far apart, else a small rate correction) until
//!   it holds within 12 ms. `DECIDE_S` before the fade, if it holds, the main page gets `overlap` and the
//!   voice `swap`: when the main page reaches the fade (W) it goes down in `SWAP_MS` while the voice comes
//!   up (the same audio, so linear holds the level); then the main page seeks to its end, the site moves
//!   on, the next track fades in, and the voice fades out with the transition's effects. Anything not
//!   ready, not in step or odd before W (an ad, a pause, a seek, another track, a sound effect on) closes
//!   the voice and the main page does the one-player fade.
//! - After the swap the voice only plays while the main page shows no ad: Rust watches the player's
//!   state every 20 ms and silences the voice at once on one. An ad on the voice closes it and turns the
//!   voice off until the app restarts (it never plays one); a voice that stops or errors by itself while
//!   playing counts as the account refusing a second playback, and after two the voice stays off until
//!   the app restarts. One that fails before it plays only falls back. Each track gets one attempt.
//! - Every transition is logged under `transition` (diag.rs): which mode played and why, with the clocks'
//!   skew and round trips, when the voice was ready and in step, and its offset at the swap.

use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::webview::{NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl};

use crate::audio::{CURVES, EQ_MAX_DB};
use crate::player::{Player, valid_id};
use crate::settings::Live;

const LABEL: &str = "deck";
const SCRIPT: &str = include_str!("../injections/deck/page.js");
const MAIN_POLL: &str = "window.__feathertune?.audio?.deck?.() ?? null";
/// The voice opens when the fade is this far (seconds of the main page's playback).
const OPEN_S: RangeInclusive<f64> = 14.0..=25.0;
/// It waits this far before the fade, starts (silent) this far before it, and must be ready by then
/// at the latest.
const PRE_ROLL_S: f64 = 5.0;
const GO_S: f64 = 5.5;
const READY_BY_S: f64 = 2.5;
/// It must hold in step this far before the fade, or the main page fades alone.
const DECIDE_S: f64 = 1.5;
/// Below this, there is no time left to hand the fade over.
const TOO_CLOSE_S: f64 = 0.8;
/// The voice stops correcting this long before the swap (as its page does).
const FREEZE_S: f64 = 0.2;
const SWAP_MS: f64 = 60.0;
const WAIT_POLL: Duration = Duration::from_millis(250);
const SYNC_POLL: Duration = Duration::from_millis(100);
/// After the swap: the player's state every slice, both pages every `PAGES_EVERY` slices.
const SLICE: Duration = Duration::from_millis(20);
const PAGES_EVERY: u32 = 12;
const IDLE: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// How long the main page may take to accept the overlap.
const ACCEPT_TIMEOUT: Duration = Duration::from_millis(600);
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// While waiting, how long the main page's state may not fit before the voice gives up (its fade is
/// planned again now and then).
const LEAD_GRACE: Duration = Duration::from_secs(1);
/// After the swap, the site pauses the main page briefly to change track: a longer pause is the
/// listener's. A pause once the next track has played past `STARTED_S` is the listener's at once; one on
/// the outgoing track within `ENDED_S` of its end is the end of the queue (the voice plays on).
const PAUSE_GRACE: Duration = Duration::from_millis(1500);
const STARTED_S: f64 = 0.5;
const ENDED_S: f64 = 0.5;
/// How long the echo rings on after the fade (ms), and the margin without one.
const ECHO_RING_MS: f64 = 5000.0;
const TAIL_MS: f64 = 300.0;
const MAX_REFUSALS: u32 = 2;
/// The tempo effect: only within ±6 %.
const MAX_RATE_DELTA: f64 = 0.06;
/// Round trips kept per page for its clock's offset.
const SAMPLES: usize = 16;
/// The voice's gain over the main page's volume (as in its page).
const MAX_TRIM: f64 = 4.0;

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

/// The main page's state (`audio.deck()`): its track and time at `wall` (its clock), the fade it plans
/// and the chain the voice copies. Untrusted: use it only through `clamped`.
#[derive(Deserialize)]
struct Main {
    id: Option<String>,
    t: f64,
    rate: f64,
    paused: bool,
    volume: f64,
    muted: bool,
    ad: bool,
    error: bool,
    wall: f64,
    out: Option<f64>,
    dur: Option<f64>,
    curve: Option<String>,
    echo: bool,
    filter: bool,
    bass_swap: bool,
    tempo: bool,
    to: Option<String>,
    length: Option<f64>,
    armed: bool,
    faded: bool,
    /// The page's output latency (s).
    latency: f64,
    chain: Option<Chain>,
    /// A sound effect is on (the voice does not copy it).
    fx: bool,
    /// The page took the `overlap` command, and swapped to the voice.
    overlap: bool,
    swapped: bool,
}

impl Main {
    /// Within bounds, or `None` when its time cannot be used.
    fn clamped(mut self) -> Option<Main> {
        if !(self.t.is_finite() && self.t >= 0.0 && self.wall.is_finite() && (0.25..=4.0).contains(&self.rate)) {
            return None;
        }
        self.id = self.id.filter(|id| valid_id(id));
        self.to = self.to.filter(|id| valid_id(id));
        self.volume = self.volume.clamp(0.0, 1.0);
        self.latency = self.latency.clamp(0.0, 1.0);
        self.out = self.out.filter(|o| o.is_finite() && *o >= 0.0);
        self.dur = self.dur.filter(|d| *d > 0.0 && *d <= 60.0);
        if self.out.is_none() || self.dur.is_none() {
            self.out = None;
            self.dur = None;
        }
        self.curve = self.curve.filter(|c| CURVES.contains(&c.as_str()));
        self.chain = self.chain.map(Chain::clamped);
        Some(self)
    }
}

/// The main page's equalizer gains (dB), compressor and trim, for the voice.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
struct Chain {
    eq: Vec<f64>,
    comp: Option<Comp>,
    trim: f64,
}

impl Chain {
    fn clamped(self) -> Chain {
        let eq = if self.eq.len() == 10 {
            self.eq.iter().map(|g| g.clamp(-EQ_MAX_DB, EQ_MAX_DB)).collect()
        } else {
            Vec::new()
        };
        Chain { eq, comp: self.comp.map(Comp::clamped), trim: self.trim.clamp(0.0, MAX_TRIM) }
    }
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
struct Comp {
    threshold: f64,
    knee: f64,
    ratio: f64,
    attack: f64,
    release: f64,
}

impl Comp {
    fn clamped(self) -> Comp {
        Comp {
            threshold: self.threshold.clamp(-100.0, 0.0),
            knee: self.knee.clamp(0.0, 40.0),
            ratio: self.ratio.clamp(1.0, 20.0),
            attack: self.attack.clamp(0.0, 1.0),
            release: self.release.clamp(0.0, 1.0),
        }
    }
}

/// The voice's state (`__feathertuneDeck.poll()` / `sync()`).
#[derive(Deserialize)]
struct Voice {
    id: Option<String>,
    ad: bool,
    /// The page has no player yet.
    loading: bool,
    ready: bool,
    started: bool,
    paused: bool,
    ended: bool,
    wall: f64,
    /// What the page says is wrong (untrusted: only logged, see `clean`).
    error: Option<String>,
    /// Why the browser would not play (the same).
    blocked: Option<String>,
    /// How far it is from the main page (ms, heard), and whether it held within 12 ms lately.
    offset_ms: Option<f64>,
    locked: bool,
}

enum Outcome {
    Done,
    /// Back to one player for this transition.
    Fallback(&'static str),
    Refused(&'static str),
    AdOnVoice,
}

/// A page's clock against Rust's: its wall clock minus the midpoint of the read (ms), from the
/// fastest of the last few round trips (the midpoint is the most certain there).
#[derive(Default)]
struct Offset(VecDeque<(f64, f64)>);

impl Offset {
    fn add(&mut self, rtt: f64, offset: f64) {
        if !(rtt.is_finite() && offset.is_finite()) {
            return;
        }
        self.0.push_back((rtt, offset));
        if self.0.len() > SAMPLES {
            self.0.pop_front();
        }
    }
    /// (offset, its round trip).
    fn best(&self) -> Option<(f64, f64)> {
        self.0.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map(|&(rtt, offset)| (offset, rtt))
    }
}

/// Both pages' clocks, measured on every read.
struct Clocks {
    origin: Instant,
    main: Offset,
    voice: Offset,
}

impl Clocks {
    fn new() -> Clocks {
        Clocks { origin: Instant::now(), main: Offset::default(), voice: Offset::default() }
    }
    fn now_ms(&self) -> f64 {
        self.origin.elapsed().as_secs_f64() * 1000.0
    }
    /// Runs `js` and parses its result, with the midpoint and round trip of the read (ms).
    fn timed<T: DeserializeOwned>(&self, webview: &Webview, js: &str) -> Option<(T, f64, f64)> {
        let sent = self.now_ms();
        let value = read(webview, js)?;
        let back = self.now_ms();
        Some((value, (sent + back) / 2.0, back - sent))
    }
    fn main(&mut self, site: &Webview) -> Option<Main> {
        let (a, mid, rtt) = self.timed::<Main>(site, MAIN_POLL)?;
        let a = a.clamped()?;
        self.main.add(rtt, a.wall - mid);
        Some(a)
    }
    fn voice(&mut self, voice: &Webview, js: &str) -> Option<Voice> {
        let (v, mid, rtt) = self.timed::<Voice>(voice, js)?;
        if !v.wall.is_finite() {
            return None;
        }
        self.voice.add(rtt, v.wall - mid);
        Some(v)
    }
    /// What to add to a time of the main page's clock to get the voice's.
    fn skew(&self) -> Option<f64> {
        skew(&self.main, &self.voice)
    }
    /// The main page's clock now.
    fn main_now(&self) -> Option<f64> {
        self.main.best().map(|(offset, _)| self.now_ms() + offset)
    }
}

fn skew(main: &Offset, voice: &Offset) -> Option<f64> {
    Some(voice.best()?.0 - main.best()?.0)
}

/// Seconds (wall time) from the main page's state until its fade.
fn until_fade(a: &Main) -> Option<f64> {
    a.out.map(|out| (out - a.t) / a.rate)
}

/// When the main page reaches its fade, on its own clock (ms).
fn swap_time(a: &Main) -> Option<f64> {
    until_fade(a).map(|left| a.wall + left * 1000.0)
}

/// The tempo effect's rate for the incoming track: the outgoing track's tempo, when both are measured
/// and close enough.
fn tempo_rate(on: bool, from: Option<f64>, to: Option<f64>) -> f64 {
    match (from, to) {
        (Some(a), Some(b)) if on && a > 0.0 && b > 0.0 && ((a / b) - 1.0).abs() <= MAX_RATE_DELTA => a / b,
        _ => 1.0,
    }
}

/// Why the main page cannot hand its fade of `id` over now, if it cannot.
fn blocker(a: &Main, id: &str) -> Option<&'static str> {
    if a.ad {
        Some("an ad on the main page")
    } else if a.error {
        Some("the main page shows an error")
    } else if a.paused {
        Some("the main page paused")
    } else if a.id.as_deref() != Some(id) {
        Some("another track started")
    } else if a.fx {
        Some("a sound effect is on")
    } else if a.out.is_none() {
        Some("the main page changed")
    } else {
        None
    }
}

/// The main page's volume, for the voice.
fn level(a: &Main) -> serde_json::Value {
    json!({ "volume": a.volume, "muted": a.muted })
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut last_fade = None;
        // The last track the voice was tried on: each one gets a single attempt.
        let mut tried: Option<String> = None;
        loop {
            std::thread::sleep(IDLE);
            if !app.state::<Live>().plugin("crossfade") {
                continue;
            }
            let Some(site) = app.get_webview("site") else { continue };
            let main = read::<Main>(&site, MAIN_POLL).and_then(Main::clamped);
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
            let Some(id) = a.id.clone() else { continue };
            if !until_fade(&a).is_some_and(|left| OPEN_S.contains(&left)) || tried.as_deref() == Some(&id) {
                continue;
            }
            match blocker(&a, &id) {
                None => {}
                Some(why) if a.fx => {
                    tried = Some(id);
                    finished(false, why, format!("one player this time ({why})"));
                    continue;
                }
                Some(_) => continue,
            }
            tried = Some(id.clone());
            log(format_args!("voice opens {:.1} s before the fade", until_fade(&a).unwrap_or(0.0)));
            match mix(&app, &site, &id) {
                Outcome::Done => finished(true, "", "overlap: the voice played the end of the track".into()),
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

/// One transition through the voice, on the track `id` the main page plays.
fn mix(app: &AppHandle, site: &Webview, id: &str) -> Outcome {
    let Some(voice) = open(app, id) else { return Outcome::Fallback("the voice did not open") };
    let mut swapped = false;
    let outcome = run(app, site, &voice, id, &mut swapped);
    if !matches!(outcome, Outcome::Done) {
        let _ = voice.eval("window.__feathertuneDeck?.stop()");
        // Before the swap the main page still has the end of the track: back to its own fade.
        if !swapped {
            command(app, "overlap_off", json!(true));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = voice.close();
    outcome
}

/// What the voice's state says is wrong once it plays, if anything.
fn voice_fault(v: &Voice) -> Option<Outcome> {
    if v.ad {
        return Some(Outcome::AdOnVoice);
    }
    if let Some(why) = &v.blocked {
        log(format_args!("the voice could not play: {}", clean(why)));
        return Some(Outcome::Fallback("the voice could not play"));
    }
    if let Some(error) = &v.error {
        log(format_args!("voice error: {}", clean(error)));
        return Some(Outcome::Refused("the voice stopped by itself"));
    }
    if !v.started {
        return Some(Outcome::Fallback("the voice did not start"));
    }
    (v.paused && !v.ended).then_some(Outcome::Refused("the voice stopped by itself"))
}

fn run(app: &AppHandle, site: &Webview, voice: &Webview, id: &str, swapped: &mut bool) -> Outcome {
    let mut clocks = Clocks::new();
    let mut lost: Option<Instant> = None;
    let mut ready_at = None;

    // Waiting for the fade, with the voice loading and waiting PRE_ROLL_S before it.
    let a = loop {
        std::thread::sleep(WAIT_POLL);
        let Some(a) = clocks.main(site) else { return Outcome::Fallback("no answer from the page") };
        let (Some(out), Some(left)) = (a.out, until_fade(&a)) else {
            if lost.get_or_insert_with(Instant::now).elapsed() < LEAD_GRACE {
                continue;
            }
            return Outcome::Fallback("the main page changed");
        };
        if let Some(why) = blocker(&a, id) {
            if lost.get_or_insert_with(Instant::now).elapsed() < LEAD_GRACE {
                continue;
            }
            log(format_args!("main page: on {:?}, fade at {:?}, paused: {}, ad: {}", a.id, a.out, a.paused, a.ad));
            return Outcome::Fallback(why);
        }
        lost = None;
        let target = (out - PRE_ROLL_S).max(0.0);
        let v = clocks.voice(voice, &format!("window.__feathertuneDeck?.poll({target}) ?? null"));
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
        let ready = v.as_ref().is_some_and(|v| v.ready && v.id.as_deref() == Some(id));
        if ready && ready_at.is_none() {
            ready_at = Some(left);
        }
        if left > GO_S {
            continue;
        }
        if ready {
            break a;
        }
        if left > READY_BY_S {
            continue;
        }
        return Outcome::Fallback(match &v {
            None => "the voice did not answer",
            Some(v) if v.loading => "the voice was still loading",
            Some(_) => "the voice was not ready in time",
        });
    };

    // Go: the voice plays (silent) from where it waits, when the main page gets there.
    let Some(skew) = clocks.skew() else { return Outcome::Fallback("no answer from a page") };
    let target = (a.out.unwrap_or(0.0) - PRE_ROLL_S).max(0.0);
    let play_ms = a.wall + (target - a.t) / a.rate * 1000.0 + skew;
    let mut sent_level = level(&a);
    let go = json!({ "play_ms": play_ms, "level": sent_level, "chain": a.chain });
    if read::<bool>(voice, &format!("window.__feathertuneDeck?.go({go}) ?? false")) != Some(true) {
        return Outcome::Fallback("the voice did not start");
    }

    // In step until the decision, then until just before the swap.
    let mut locked_at = None;
    let mut plan: Option<(f64, f64)> = None;
    let mut offset_ms = None;
    loop {
        std::thread::sleep(SYNC_POLL);
        let Some(a) = clocks.main(site) else { return Outcome::Fallback("no answer from the page") };
        if let Some(why) = blocker(&a, id) {
            return Outcome::Fallback(why);
        }
        if plan.is_some() && !a.overlap {
            return Outcome::Fallback("the page dropped the overlap");
        }
        let Some(skew) = clocks.skew() else { return Outcome::Fallback("no answer from a page") };
        let sync = json!({ "t": a.t, "wall_ms": a.wall + skew, "rate": a.rate, "latency": a.latency });
        let Some(v) = clocks.voice(voice, &format!("window.__feathertuneDeck?.sync({sync}) ?? null")) else {
            return Outcome::Fallback("the voice did not answer");
        };
        if let Some(fault) = voice_fault(&v) {
            return fault;
        }
        offset_ms = v.offset_ms.filter(|o| o.is_finite()).or(offset_ms);
        let now_level = level(&a);
        if now_level != sent_level {
            let _ = voice.eval(format!("window.__feathertuneDeck?.volume({now_level})"));
            sent_level = now_level;
        }
        let Some(left) = until_fade(&a) else { return Outcome::Fallback("the main page changed") };
        if v.locked && locked_at.is_none() {
            locked_at = Some(left);
        }
        if let Some((w, _)) = plan {
            // The voice froze FREEZE_S before W by itself; from here on the swap is scheduled.
            if a.wall >= w - (FREEZE_S * 1000.0 + SYNC_POLL.as_secs_f64() * 1000.0) {
                break;
            }
            continue;
        }
        if left > DECIDE_S {
            continue;
        }
        if left < TOO_CLOSE_S {
            return Outcome::Fallback("the fade was too close to start the voice");
        }
        if !v.locked {
            log(format_args!("the voice did not lock: {offset_ms:?} ms off"));
            return Outcome::Fallback("the voice did not lock");
        }
        // The decision: the main page swaps to the voice at W.
        let Some(w) = swap_time(&a) else { return Outcome::Fallback("the main page changed") };
        let dur_ms = a.dur.unwrap_or(0.0) * 1000.0 / a.rate;
        let store = app.state::<crate::audio_facts::Store>();
        let bpm_from = store.load(id).and_then(|f| f.bpm).filter(|b| b.is_finite() && *b > 0.0);
        let bpm_to = a.to.as_deref().and_then(|to| store.load(to)).and_then(|f| f.bpm);
        let beat_ms = bpm_from.map_or(500.0, |b| 60_000.0 / b);
        let rate = tempo_rate(a.tempo, bpm_from, bpm_to);
        command(
            app,
            "overlap",
            json!({ "from": id, "at_ms": w, "swap_ms": SWAP_MS, "dur_ms": dur_ms, "rate": rate,
            "bass_swap": a.bass_swap, "beat_ms": beat_ms }),
        );
        if !accepted(site) {
            return Outcome::Fallback("the page did not accept the overlap");
        }
        let swap = json!({ "at_ms": w + skew, "swap_ms": SWAP_MS, "dur_ms": dur_ms, "curve": a.curve,
            "echo": a.echo, "filter": a.filter, "bass_swap": a.bass_swap, "beat_ms": beat_ms });
        if read::<bool>(voice, &format!("window.__feathertuneDeck?.swap({swap}) ?? false")) != Some(true) {
            return Outcome::Fallback("the voice did not answer");
        }
        plan = Some((w, dur_ms + if a.echo { ECHO_RING_MS } else { TAIL_MS }));
    }
    let Some((w, tail_ms)) = plan else { return Outcome::Fallback("the main page changed") };
    let (main_rtt, voice_rtt) = (clocks.main.best().map(|b| b.1), clocks.voice.best().map(|b| b.1));
    log(format_args!(
        "swap: skew {:.1} ms, round trips {main_rtt:.1?}/{voice_rtt:.1?} ms, ready {ready_at:.1?} s and in step \
         {locked_at:.1?} s before the fade, {offset_ms:?} ms off",
        clocks.skew().unwrap_or(0.0)
    ));

    // After the swap: the voice plays the end, silenced at once on an ad on the main page.
    let mut rx = app.state::<Player>().subscribe();
    let end = w + SWAP_MS + tail_ms;
    let mut paused_since: Option<Instant> = None;
    let poll = "window.__feathertuneDeck?.poll() ?? null";
    for slice in 0u32.. {
        std::thread::sleep(SLICE);
        loop {
            use tokio::sync::broadcast::error::TryRecvError;
            match rx.try_recv() {
                Ok(event) if event.state().ad => return Outcome::Fallback("an ad on the main page"),
                Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        if slice % PAGES_EVERY != 0 {
            continue;
        }
        let past_end = clocks.main_now().is_some_and(|now| now >= end);
        let Some(v) = clocks.voice(voice, poll) else {
            if past_end {
                return Outcome::Done;
            }
            return Outcome::Fallback("the voice did not answer");
        };
        if v.ad {
            return Outcome::AdOnVoice;
        }
        if past_end {
            return Outcome::Done;
        }
        if !v.ended
            && let Some(fault) = voice_fault(&v)
        {
            return fault;
        }
        let Some(a) = clocks.main(site) else {
            // The main page is changing track: the voice goes on, the deadline holds.
            continue;
        };
        if a.ad {
            return Outcome::Fallback("an ad on the main page");
        }
        *swapped |= a.swapped;
        if !*swapped {
            if !a.overlap {
                return Outcome::Fallback("the page dropped the overlap");
            }
            if let Some(why) = blocker(&a, id) {
                return Outcome::Fallback(why);
            }
        } else if a.paused || a.error {
            match after_swap_pause(&a, id) {
                Pause::Listener => return Outcome::Fallback("the main page paused"),
                // The end of the queue: the voice plays its tail to the deadline.
                Pause::Ended => paused_since = None,
                Pause::Unsure => {
                    if paused_since.get_or_insert_with(Instant::now).elapsed() > PAUSE_GRACE {
                        return Outcome::Fallback("the main page paused");
                    }
                }
            }
        } else {
            paused_since = None;
        }
        let now_level = level(&a);
        if now_level != sent_level {
            let _ = voice.eval(format!("window.__feathertuneDeck?.volume({now_level})"));
            sent_level = now_level;
        }
    }
    Outcome::Done
}

/// What a pause (or an error) of the main page after the swap means.
#[derive(Debug, PartialEq)]
enum Pause {
    /// The next track already played on the main page: the listener paused it.
    Listener,
    /// The main page stayed on the outgoing track `id` at its end: nothing follows in the queue.
    Ended,
    /// The site may be changing track (still on the outgoing track, or at the start of the next one).
    Unsure,
}

fn after_swap_pause(a: &Main, id: &str) -> Pause {
    match a.id.as_deref() {
        Some(now) if now == id => {
            let ended = a.length.is_some_and(|l| l.is_finite() && a.t >= l - ENDED_S);
            if ended { Pause::Ended } else { Pause::Unsure }
        }
        Some(_) if a.paused && !a.error && a.t > STARTED_S => Pause::Listener,
        _ => Pause::Unsure,
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

/// The hidden voice webview on track `id`, with the same profile and browser arguments as the site
/// (WebView2 needs both to share one browser process) and only the `deck` script.
fn open(app: &AppHandle, id: &str) -> Option<Webview> {
    if let Some(old) = app.get_webview(LABEL) {
        let _ = old.close();
    }
    let window = app.get_window("main")?;
    let live = app.state::<Live>();
    let url: Url = format!("https://music.youtube.com/watch?v={id}").parse().ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn main_state(extra: serde_json::Value) -> Main {
        let mut base = json!({
            "id": "abcdefghijk", "t": 100.0, "rate": 1.0, "paused": false, "volume": 0.5, "muted": false,
            "ad": false, "error": false, "wall": 1_000_000.0, "out": 110.0, "dur": 6.0, "curve": "linear",
            "echo": false, "filter": false, "bass_swap": false, "tempo": false, "to": null, "length": 120.0,
            "armed": false, "faded": false, "latency": 0.02, "chain": null, "fx": false, "overlap": false,
            "swapped": false,
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn offset_keeps_the_fastest_round_trip() {
        let mut o = Offset::default();
        o.add(30.0, 12.0);
        o.add(4.0, 7.0);
        o.add(f64::NAN, 1.0);
        o.add(9.0, 9.0);
        assert_eq!(o.best(), Some((7.0, 4.0)));
        for _ in 0..SAMPLES {
            o.add(20.0, 3.0);
        }
        // The fast one is too old now.
        assert_eq!(o.best(), Some((3.0, 20.0)));
    }

    #[test]
    fn skew_moves_main_times_onto_the_voice() {
        let (mut main, mut voice) = (Offset::default(), Offset::default());
        assert_eq!(skew(&main, &voice), None);
        main.add(2.0, 1000.0);
        voice.add(2.0, 1250.0);
        // A moment at 5000 on the main page's clock is 5250 on the voice's.
        assert_eq!(skew(&main, &voice), Some(250.0));
    }

    #[test]
    fn a_pause_after_the_swap_is_told_apart() {
        let id = "abcdefghijk";
        let next = "kjihgfedcba";
        let pause = |extra: serde_json::Value| after_swap_pause(&main_state(extra), id);
        // The next track played: the listener's pause.
        assert_eq!(pause(json!({ "id": next, "t": 3.0, "paused": true })), Pause::Listener);
        // At its start, or an error: maybe the site changing track.
        assert_eq!(pause(json!({ "id": next, "t": 0.0, "paused": true })), Pause::Unsure);
        assert_eq!(pause(json!({ "id": next, "t": 3.0, "error": true })), Pause::Unsure);
        assert_eq!(pause(json!({ "id": null, "t": 3.0, "paused": true })), Pause::Unsure);
        // Still on the outgoing track: before its end maybe changing track, at its end the queue ended.
        assert_eq!(pause(json!({ "t": 110.0, "paused": true })), Pause::Unsure);
        assert_eq!(pause(json!({ "t": 119.8, "paused": true })), Pause::Ended);
        assert_eq!(pause(json!({ "t": 119.8, "paused": true, "length": null })), Pause::Unsure);
    }

    #[test]
    fn swap_time_follows_the_rate() {
        let a = main_state(json!({}));
        assert_eq!(until_fade(&a), Some(10.0));
        assert_eq!(swap_time(&a), Some(1_010_000.0));
        let a = main_state(json!({ "rate": 2.0 }));
        assert_eq!(swap_time(&a), Some(1_005_000.0));
        let a = main_state(json!({ "out": null }));
        assert_eq!(swap_time(&a), None);
    }

    #[test]
    fn tempo_only_when_close() {
        assert_eq!(tempo_rate(true, Some(126.0), Some(120.0)), 1.05);
        assert_eq!(tempo_rate(false, Some(126.0), Some(120.0)), 1.0);
        assert_eq!(tempo_rate(true, Some(140.0), Some(120.0)), 1.0);
        assert_eq!(tempo_rate(true, None, Some(120.0)), 1.0);
        assert_eq!(tempo_rate(true, Some(120.0), Some(0.0)), 1.0);
    }

    #[test]
    fn main_is_clamped() {
        assert!(main_state(json!({ "rate": 0.0 })).clamped().is_none());
        assert!(main_state(json!({ "t": -1.0 })).clamped().is_none());
        let a = main_state(json!({ "id": "../x", "volume": 3.0, "dur": 600.0, "curve": "nope", "latency": 9.0 }))
            .clamped()
            .unwrap();
        assert_eq!(a.id, None);
        assert_eq!(a.volume, 1.0);
        assert_eq!(a.latency, 1.0);
        assert_eq!((a.out, a.dur), (None, None));
        assert_eq!(a.curve, None);
        assert_eq!(blocker(&a, "abcdefghijk"), Some("another track started"));
        let a = main_state(json!({ "fx": true })).clamped().unwrap();
        assert_eq!(blocker(&a, "abcdefghijk"), Some("a sound effect is on"));
        assert_eq!(blocker(&main_state(json!({})).clamped().unwrap(), "abcdefghijk"), None);
    }

    #[test]
    fn chain_is_clamped() {
        let comp = Comp { threshold: 5.0, knee: 99.0, ratio: 0.0, attack: -1.0, release: 2.0 };
        let c = Chain { eq: vec![20.0; 10], comp: Some(comp), trim: 9.0 }.clamped();
        assert_eq!(c.eq, vec![EQ_MAX_DB; 10]);
        assert_eq!(c.comp, Some(Comp { threshold: 0.0, knee: 40.0, ratio: 1.0, attack: 0.0, release: 1.0 }));
        assert_eq!(c.trim, MAX_TRIM);
        assert!(Chain { eq: vec![1.0; 3], comp: None, trim: 1.0 }.clamped().eq.is_empty());
    }
}
