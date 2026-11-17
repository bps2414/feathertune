//! Stage (plugin `stage`): a directed full-screen show over the synced lyrics. Rust writes the script
//! once per track (`direct`, pure and tested here) and the `stage` page module (injections/stage/) only
//! plays it, in a fixed layer over the site, on the compositor.
//!
//! - The director reads the lines (word times, ends), the track analysis' facts when the track was
//!   measured (tempo, first beat, the loudness envelope), the title, artist, album, the next track and a
//!   seed (the video id), and writes sections: an opening up to the first line, blocks of lines (a gap
//!   over `max(3 s, 7 beats)`, 4.5 s without a tempo, or a change between repeated and unrepeated lines
//!   starts a new one), each a chorus (half its lines sung elsewhere too), a close-up (energy under 30%),
//!   an editorial stack (over 55%) or the one the last verse was not; an instrumental where the gap after
//!   a block is over `max(5 s, 8 beats)`; credits after the last line. Cuts land on a bar line when one is
//!   within 0.75 beat. The energy is the low band × 0.6 plus the mid × 0.4, between the song's own 10th
//!   and 90th percentiles; it is also sent at 10 Hz (`light`) for the page's light and relief. Without
//!   facts the lyrics alone decide; without lyrics it is an opening, an instrumental and the credits.
//! - Opened from the title bar button or the tray: the window goes fullscreen (`titlebar::set_fullscreen`),
//!   the focus goes to the page and the page gets `{ open: true, script, lines, ... }` through
//!   `injections::send`. Closed by Esc (titlebar.rs, WebView2's `AcceleratorKeyPressed`), by the page's
//!   own fullscreen ending (the HUD's leave button enters and leaves it, so the existing
//!   `ContainsFullScreenElementChanged` path restores the window), by hiding or minimizing the window,
//!   by game mode and by the plugin's switch. The page asks Rust for nothing: no new command or permission.
//! - The script is written again while open when the track, its lines, its facts or its background change.
//! - The display font, Bricolage Grotesque (SIL Open Font License, `injections/stage/OFL.txt`), is
//!   compiled in and sent once as bytes (`stageFont`); the page loads it with the FontFace API.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;

use crate::audio_facts::Facts;
use crate::i18n::t;
use crate::lyrics::Line;
use crate::player::{Event, Player};
use crate::settings::Live;

/// The variable latin subset at the 96 pt optical size, weights 200 to 800.
const FONT: &[u8] = include_bytes!("../injections/stage/bricolage-grotesque-latin-wght.woff2");
/// A cut moves to a bar line at most this share of a beat away.
const SNAP_BEATS: f64 = 0.75;
/// A section starts this long before its first line.
const LEAD_MS: f64 = 150.0;
/// Under these an opening or the credits are not worth a scene of their own.
const MIN_OPENING_MS: f64 = 1500.0;
const MIN_CREDITS_MS: f64 = 3000.0;
/// At most 15 minutes of light at 10 Hz (the envelope's own limit).
const MAX_LIGHT: usize = 9000;

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Opening,
    Close,
    Editorial,
    Chorus,
    Instrumental,
    Credits,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Right,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Section {
    pub kind: Kind,
    pub from_ms: u32,
    pub to_ms: u32,
    /// The lines it shows (indices into the lyrics' lines); empty outside the text scenes.
    pub lines: Vec<usize>,
    pub align: Align,
    /// 1 for the first chorus, 2 for the second...; 0 for anything else.
    pub chorus_no: u32,
    /// Why the director chose it, as a code (for the diagnostics and the tests; not shown).
    pub why: &'static str,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
pub struct Grid {
    pub bpm: f64,
    pub offset_ms: f64,
}

#[derive(Serialize, Clone, PartialEq, Debug, Default)]
pub struct Next {
    pub title: String,
    pub artist: String,
}

#[derive(Serialize, Clone, PartialEq, Debug, Default)]
pub struct Credits {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub next: Option<Next>,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Script {
    pub duration_ms: u32,
    /// Contiguous, from 0 to `duration_ms`.
    pub sections: Vec<Section>,
    pub grid: Option<Grid>,
    /// The energy at 10 Hz, 0 to 255 over the song's own range; empty without facts.
    pub light: Vec<u8>,
    /// Per line: how many times its text was sung before (the chorus' echoes).
    pub echoes: Vec<u8>,
    /// Per line: when it stops being sung (its own end, else the next line's start).
    pub ends: Vec<u32>,
    pub credits: Credits,
}

/// What the director knows of the track besides its lines and facts.
pub struct Track<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    /// 0: unknown.
    pub duration_ms: u32,
    /// The video id (or the title): the same seed always gives the same script.
    pub seed: &'a str,
    pub next: Option<Next>,
}

/// A line's text for comparing repeats: lowercase, without accents, punctuation or extra spaces.
fn norm(text: &str) -> String {
    let fold = |c: char| match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        c => c,
    };
    let kept: String = text
        .chars()
        .flat_map(char::to_lowercase)
        .map(fold)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// When line `i` stops being sung: its own end, else the next line's start, else 4 s on.
pub fn line_end(lines: &[Line], i: usize, duration: f64) -> f64 {
    let l = &lines[i];
    let ms = f64::from(l.ms);
    if l.end > l.ms {
        return f64::from(l.end);
    }
    match lines[i + 1..].iter().find(|n| n.ms > l.ms) {
        Some(n) => f64::from(n.ms),
        None => (ms + 4000.0).min(duration.max(ms + 1000.0)),
    }
}

/// The envelope's energy (see the module doc), per 100 ms bin, 0 to 1.
struct Energy(Vec<f64>);

impl Energy {
    fn of(facts: Option<&Facts>) -> Energy {
        let bytes =
            facts.and_then(|f| base64::engine::general_purpose::STANDARD.decode(&f.envelope).ok()).unwrap_or_default();
        let level: Vec<f64> = bytes
            .as_chunks::<3>()
            .0
            .iter()
            .map(|[low, mid, _]| (f64::from(*low) * 0.6 + f64::from(*mid) * 0.4) / 255.0)
            .collect();
        if level.len() < 10 {
            return Energy(Vec::new());
        }
        let mut sorted = level.clone();
        sorted.sort_by(f64::total_cmp);
        let (p10, p90) = (sorted[sorted.len() / 10], sorted[sorted.len() * 9 / 10]);
        let span = if p90 - p10 > 1e-6 { p90 - p10 } else { 1.0 };
        Energy(level.iter().map(|x| ((x - p10) / span).clamp(0.0, 1.0)).collect())
    }

    /// The mean from `a` to `z` (ms); None without facts or outside them.
    fn between(&self, a: f64, z: f64) -> Option<f64> {
        let from = (a.max(0.0) / 100.0).floor() as usize;
        let to = ((z / 100.0).ceil().max(0.0) as usize).min(self.0.len());
        (from < to).then(|| self.0[from..to].iter().sum::<f64>() / (to - from) as f64)
    }

    fn light(&self) -> Vec<u8> {
        self.0.iter().take(MAX_LIGHT).map(|e| (e * 255.0).round() as u8).collect()
    }
}

/// The seeded choices (left or right, which verse look starts): the same seed, the same script.
struct Rng(u32);

impl Rng {
    fn new(seed: &str) -> Rng {
        Rng(seed.bytes().fold(7u32, |a, c| a.wrapping_mul(31).wrapping_add(u32::from(c))))
    }

    fn coin(&mut self) -> bool {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        self.0 >> 31 == 1
    }
}

fn section(kind: Kind, from: f64, to: f64, why: &'static str) -> Section {
    Section {
        kind,
        from_ms: from.max(0.0).round() as u32,
        to_ms: to.max(0.0).round() as u32,
        lines: Vec::new(),
        align: Align::Left,
        chorus_no: 0,
        why,
    }
}

/// Writes the script of a track (see the module doc).
pub fn direct(lines: &[Line], facts: Option<&Facts>, track: &Track) -> Script {
    let last_line = lines.iter().map(|l| l.ms.max(l.end)).max().map_or(0.0, f64::from);
    let duration = if track.duration_ms > 0 {
        f64::from(track.duration_ms)
    } else if let Some(f) = facts {
        f.duration_s * 1000.0
    } else {
        last_line + 5000.0
    };
    let bpm = facts.and_then(|f| f.bpm).filter(|b| b.is_finite() && *b > 0.0);
    let beat = bpm.map(|b| 60_000.0 / b);
    let offset = facts.and_then(|f| f.beat_offset_ms).unwrap_or(0.0);
    let snap = |t: f64| match beat {
        Some(beat) => {
            let bar = beat * 4.0;
            let s = offset + ((t - offset) / bar).round() * bar;
            if (s - t).abs() < beat * SNAP_BEATS && s >= 0.0 { s } else { t }
        }
        None => t,
    };
    let energy = Energy::of(facts);
    let ends: Vec<f64> = (0..lines.len()).map(|i| line_end(lines, i, duration)).collect();
    let texts: Vec<String> = lines.iter().map(|l| norm(&l.text)).collect();
    let sung: Vec<usize> = (0..lines.len()).filter(|&i| !texts[i].is_empty()).collect();
    let mut count = std::collections::HashMap::<&str, u32>::new();
    let mut echoes = vec![0u8; lines.len()];
    for &i in &sung {
        let n = count.entry(texts[i].as_str()).or_default();
        echoes[i] = (*n).min(9) as u8;
        *n += 1;
    }
    let repeated = |i: usize| count.get(texts[i].as_str()).is_some_and(|n| *n >= 2);
    let mut rng = Rng::new(track.seed);

    let mut out = Vec::new();
    if sung.is_empty() {
        // No lyrics: the cover's opening, an instrumental with the relief, the credits.
        let head = snap((duration * 0.2).min(12_000.0));
        let tail = snap(duration - (duration * 0.2).min(15_000.0));
        out.push(section(Kind::Opening, 0.0, head, "no_lyrics"));
        out.push(section(Kind::Instrumental, head, tail, "no_lyrics"));
        out.push(section(Kind::Credits, tail, duration, "no_lyrics"));
    } else {
        // 1. Blocks of lines.
        let gap_max = beat.map_or(4500.0, |b| (b * 7.0).max(3000.0));
        let mut blocks: Vec<Vec<usize>> = Vec::new();
        for (k, &i) in sung.iter().enumerate() {
            let new = match k.checked_sub(1).map(|p| sung[p]) {
                None => true,
                Some(p) => f64::from(lines[i].ms) - ends[p] > gap_max || repeated(p) != repeated(i),
            };
            if new {
                blocks.push(Vec::new());
            }
            blocks.last_mut().expect("a block").push(i);
        }
        // 2. The opening, until the first line.
        let first = snap(f64::from(lines[sung[0]].ms) - LEAD_MS);
        if first >= MIN_OPENING_MS {
            out.push(section(Kind::Opening, 0.0, first, "before_first"));
        }
        // 3. Each block a scene, and an instrumental in a long gap after it.
        let mut last_verse = if rng.coin() { Kind::Editorial } else { Kind::Close };
        let mut chorus_no = 0;
        let instrumental = beat.map_or(5000.0, |b| (b * 8.0).max(5000.0));
        for (k, block) in blocks.iter().enumerate() {
            let from = if k == 0 { first.max(0.0) } else { snap(f64::from(lines[block[0]].ms) - LEAD_MS) };
            let end = ends[*block.last().expect("a line")];
            let next = blocks.get(k + 1).map(|b| f64::from(lines[b[0]].ms));
            let chorus = block.iter().filter(|&&i| repeated(i)).count() * 2 >= block.len();
            let e = energy.between(from, end);
            let (kind, why) = if chorus {
                (Kind::Chorus, "repeated")
            } else {
                match e {
                    Some(e) if e < 0.3 => (Kind::Close, "low_energy"),
                    Some(e) if e > 0.55 => (Kind::Editorial, "high_energy"),
                    _ => {
                        let kind = if last_verse == Kind::Close { Kind::Editorial } else { Kind::Close };
                        (kind, if e.is_some() { "alternate" } else { "no_analysis" })
                    }
                }
            };
            if chorus {
                chorus_no += 1;
            } else {
                last_verse = kind;
            }
            let mut s = section(kind, from, end, why);
            s.lines = block.clone();
            s.align = if rng.coin() { Align::Right } else { Align::Left };
            s.chorus_no = if chorus { chorus_no } else { 0 };
            match next {
                Some(next) if next - end > instrumental => {
                    let at = snap(end + beat.unwrap_or(500.0));
                    s.to_ms = at.round() as u32;
                    out.push(s);
                    out.push(section(Kind::Instrumental, at, snap(next - LEAD_MS), "gap"));
                }
                Some(next) => {
                    s.to_ms = snap(next - LEAD_MS).max(0.0).round() as u32;
                    out.push(s);
                }
                None => out.push(s),
            }
        }
        // 4. The credits, after the last line.
        let last_end = ends[*sung.last().expect("a line")];
        let credits = snap(last_end + beat.unwrap_or(1000.0));
        if duration - credits >= MIN_CREDITS_MS {
            if let Some(s) = out.last_mut() {
                s.to_ms = credits.round() as u32;
            }
            out.push(section(Kind::Credits, credits, duration, "after_last"));
        }
    }
    let duration_ms = duration.max(1.0).round() as u32;
    Script {
        duration_ms,
        sections: tidy(out, duration_ms),
        grid: bpm.map(|bpm| Grid { bpm, offset_ms: offset }),
        light: energy.light(),
        echoes,
        ends: ends.iter().map(|e| e.round().max(0.0) as u32).collect(),
        credits: Credits {
            title: track.title.to_owned(),
            artist: track.artist.to_owned(),
            album: track.album.to_owned(),
            next: track.next.clone(),
        },
    }
}

/// Makes the sections contiguous from 0 to `duration`: each starts where the one before ends, and one
/// left with no time (a cut snapped past the next) gives its lines to the one before, or to the next one
/// when it is the first. A scene without text that gets lines becomes a close-up.
fn tidy(sections: Vec<Section>, duration: u32) -> Vec<Section> {
    let take = |s: &mut Section, lines: Vec<usize>| {
        s.lines.extend(lines);
        s.lines.sort_unstable();
        if !s.lines.is_empty() && matches!(s.kind, Kind::Opening | Kind::Instrumental | Kind::Credits) {
            s.kind = Kind::Close;
        }
    };
    let mut out: Vec<Section> = Vec::new();
    let mut carried = Vec::new();
    let mut at = 0;
    for mut s in sections {
        let to = s.to_ms.min(duration);
        if to <= at {
            match out.last_mut() {
                Some(prev) => take(prev, s.lines),
                None => carried.extend(s.lines),
            }
            continue;
        }
        take(&mut s, std::mem::take(&mut carried));
        s.from_ms = at;
        s.to_ms = to;
        at = to;
        out.push(s);
    }
    match out.last_mut() {
        Some(last) => last.to_ms = duration,
        None => {
            let mut s = section(Kind::Opening, 0.0, f64::from(duration), "no_lyrics");
            take(&mut s, carried);
            out.push(s);
        }
    }
    out
}

static APP: OnceLock<AppHandle> = OnceLock::new();
/// The plugin was on at startup, so the page has the module.
static LOADED: AtomicBool = AtomicBool::new(false);
/// `LOADED` and the switch is still on.
static AVAILABLE: AtomicBool = AtomicBool::new(false);
static OPEN: AtomicBool = AtomicBool::new(false);
static FONT_SENT: AtomicBool = AtomicBool::new(false);

pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    if !app.state::<Live>().plugin("stage") {
        return;
    }
    LOADED.store(true, Ordering::Relaxed);
    AVAILABLE.store(true, Ordering::Relaxed);
    let mut rx = app.state::<Player>().subscribe();
    std::thread::spawn(move || {
        loop {
            match rx.blocking_recv() {
                Ok(Event::Track(_)) => refresh(),
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// The title bar button and the tray item are there (the module is in the page and the switch on).
pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// The plugin's switch: off closes the stage and takes the button away until it is on again (which,
/// without the module in the page, needs a restart).
pub fn set_enabled(app: &AppHandle, on: bool) {
    if !on {
        close(app);
    }
    AVAILABLE.store(on && LOADED.load(Ordering::Relaxed), Ordering::Relaxed);
    crate::titlebar::refresh(app);
}

/// Opens the stage: the window comes back if hidden, goes fullscreen, the page gets the focus and the
/// script. Not in game mode. Any thread.
pub fn open(app: &AppHandle) {
    if !available() || crate::game_mode::is_on() || OPEN.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(window) = app.get_window("main") else {
        OPEN.store(false, Ordering::Relaxed);
        return;
    };
    if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
        crate::show_main(app);
    }
    crate::diag::log("stage", "open");
    if !FONT_SENT.swap(true, Ordering::Relaxed) {
        let data = base64::engine::general_purpose::STANDARD.encode(FONT);
        crate::injections::send(app, "stageFont", &json!({ "data": data }));
    }
    crate::titlebar::set_fullscreen(&window, true);
    if let Some(site) = app.get_webview("site") {
        let _ = site.with_webview(|wv| unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC;
            let _ = wv.controller().MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        });
    }
    crate::injections::send(app, "stage", &payload(app));
}

/// Closes the stage and gives the window its size back. Any thread.
pub fn close(app: &AppHandle) {
    if close_page(app)
        && let Some(window) = app.get_window("main")
    {
        crate::titlebar::set_fullscreen(&window, false);
    }
}

/// Closes the stage on the page only (the window was minimized: it leaves fullscreen once restored, see
/// titlebar.rs). Whether it was open.
pub fn close_page(app: &AppHandle) -> bool {
    if !OPEN.swap(false, Ordering::Relaxed) {
        return false;
    }
    crate::diag::log("stage", "closed");
    crate::injections::send(app, "stage", &json!({ "open": false }));
    true
}

/// Writes the script again (the track, its lines, its facts or its background changed); only while open.
/// Any thread.
pub fn refresh() {
    if let Some(app) = APP.get().filter(|_| is_open()) {
        crate::injections::send(app, "stage", &payload(app));
    }
}

/// What the page gets while open (see injections/stage/page.js).
fn payload(app: &AppHandle) -> Value {
    let state = app.state::<Player>().current();
    let track = state.map(|s| s.track).unwrap_or_else(|| crate::player::Track {
        id: None,
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        artwork: None,
        duration: 0.0,
    });
    let key = crate::lyrics::track_key(&track);
    let synced = app.try_state::<crate::lyrics::Lyrics>().and_then(|l| l.current()).filter(|s| s.key == key);
    let lines: &[Line] = synced.as_ref().map_or(&[], |s| &s.lines);
    let facts = track.id.as_deref().and_then(|id| app.state::<crate::audio_facts::Store>().load(id));
    let queue = app.state::<Player>().queue();
    let next = queue
        .current
        .and_then(|c| queue.items.get(c + 1))
        .map(|n| Next { title: n.title.clone(), artist: n.artist.clone() });
    let seed = track.id.clone().unwrap_or_else(|| key.clone());
    let script = direct(
        lines,
        facts.as_ref(),
        &Track {
            title: &track.title,
            artist: &track.artist,
            album: &track.album,
            duration_ms: (track.duration.max(0.0) * 1000.0).round() as u32,
            seed: &seed,
            next,
        },
    );
    let words = |l: &Line| -> Value {
        if l.words.is_empty() { Value::Null } else { l.words.iter().map(|(ms, w)| json!([ms, w])).collect() }
    };
    let page_lines: Vec<Value> = lines.iter().map(|l| json!([l.ms, l.text, words(l)])).collect();
    let subtitles = app.state::<Live>().get().stage.translation;
    let translation = (subtitles && lines.iter().any(|l| l.translation.is_some()))
        .then(|| lines.iter().map(|l| l.translation.as_deref()).collect::<Vec<_>>());
    json!({
        "open": true,
        "id": track.id,
        "script": script,
        "lines": page_lines,
        "translation": translation,
        "field": crate::ambient::field(),
        "texts": {
            "leave": t("Leave the stage"),
            "esc": t("Esc to leave"),
            "play": t("Play"),
            "pause": t("Pause"),
            "chorus": t("Chorus"),
            "instrumental": t("Instrumental"),
            "listened": t("You listened to"),
            "next": t("Up next"),
            "scenes": {
                "opening": t("Opening"),
                "close": t("Close-up"),
                "editorial": t("Editorial"),
                "chorus": t("Chorus"),
                "instrumental": t("Instrumental"),
                "credits": t("Credits"),
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ms: u32, end: u32, text: &str) -> Line {
        Line { ms, end, text: text.into(), ..Line::default() }
    }

    fn track(duration_ms: u32) -> Track<'static> {
        Track { title: "T", artist: "A", album: "B", duration_ms, seed: "abcdefghijk", next: None }
    }

    /// 120 BPM (500 ms beats, 2 s bars) from 0, with an even envelope.
    fn facts(duration_s: f64) -> Facts {
        let bins = (duration_s * 10.0) as usize;
        let bytes: Vec<u8> = (0..bins).flat_map(|b| [(b % 200) as u8 + 20, 120, 60]).collect();
        Facts {
            v: crate::audio_facts::VERSION,
            id: "abcdefghijk".into(),
            duration_s,
            bpm: Some(120.0),
            beat_offset_ms: Some(0.0),
            key: None,
            camelot: None,
            lead_in_ms: 0.0,
            tail_ms: None,
            fade_ms: None,
            envelope: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }

    /// A verse, a chorus, a long gap, the verse again, the chorus again.
    fn song() -> Vec<Line> {
        vec![
            line(10_100, 13_000, "Acordei com a cidade acesa"),
            line(13_200, 16_000, "Teu nome no vidro"),
            line(16_200, 19_000, "Luz de vidro, não apaga"),
            line(19_200, 22_000, "Dança comigo"),
            line(40_100, 43_000, "Outro verso aqui"),
            line(43_200, 46_000, "E mais um verso"),
            line(46_200, 49_000, "Luz de VIDRO nao apaga!"),
            line(49_200, 52_000, "Dança comigo"),
        ]
    }

    fn kinds(s: &Script) -> Vec<Kind> {
        s.sections.iter().map(|s| s.kind).collect()
    }

    fn assert_contiguous(s: &Script) {
        assert_eq!(s.sections.first().unwrap().from_ms, 0);
        assert_eq!(s.sections.last().unwrap().to_ms, s.duration_ms);
        for w in s.sections.windows(2) {
            assert_eq!(w[0].to_ms, w[1].from_ms, "{w:?}");
        }
        assert!(s.sections.iter().all(|s| s.to_ms > s.from_ms), "{:?}", s.sections);
    }

    #[test]
    fn repeated_lines_are_a_chorus() {
        let s = direct(&song(), None, &track(60_000));
        let choruses: Vec<&Section> = s.sections.iter().filter(|s| s.kind == Kind::Chorus).collect();
        assert_eq!(choruses.len(), 2, "{:?}", kinds(&s));
        assert_eq!((choruses[0].chorus_no, choruses[1].chorus_no), (1, 2));
        assert_eq!(choruses[0].lines, [2, 3]);
        // Accents, case and punctuation do not matter; the second time is an echo of the first.
        assert_eq!(s.echoes[6], 1);
        assert_eq!(s.echoes[2], 0);
    }

    #[test]
    fn a_long_gap_is_an_instrumental() {
        let s = direct(&song(), None, &track(60_000));
        let i = s.sections.iter().position(|s| s.kind == Kind::Instrumental).expect("an instrumental");
        let inst = &s.sections[i];
        assert!(inst.from_ms >= 22_000 && inst.to_ms <= 40_100, "{inst:?}");
        assert_eq!(s.sections[i + 1].lines[0], 4);
    }

    #[test]
    fn cuts_land_on_the_bar() {
        let s = direct(&song(), Some(&facts(60.0)), &track(60_000));
        // The first line at 10.1 s: its scene starts on the bar at 10 s, not at 9.95 s.
        assert_eq!(s.sections[0].kind, Kind::Opening);
        assert_eq!(s.sections[0].to_ms, 10_000);
        // Every scene of lines starts on a bar (the instrumental and the credits start a beat after the
        // last line's end, which only moves to a bar line within reach).
        assert!(s.sections.iter().filter(|x| !x.lines.is_empty()).all(|x| x.from_ms % 2000 == 0), "{:?}", s.sections);
        assert_eq!(s.grid, Some(Grid { bpm: 120.0, offset_ms: 0.0 }));
        assert_eq!(s.light.len(), 600);
        assert_contiguous(&s);
    }

    #[test]
    fn without_facts_the_lyrics_decide() {
        let s = direct(&song(), None, &track(60_000));
        assert!(s.grid.is_none() && s.light.is_empty());
        assert!(s.sections.iter().all(|x| !matches!(x.why, "low_energy" | "high_energy")));
        assert_eq!(s.sections[0].to_ms, 9950);
        // The verses alternate between the close-up and the editorial stack.
        let verses: Vec<Kind> = kinds(&s).into_iter().filter(|k| matches!(k, Kind::Close | Kind::Editorial)).collect();
        assert_eq!(verses.len(), 2);
        assert_ne!(verses[0], verses[1]);
        assert_contiguous(&s);
    }

    #[test]
    fn without_lyrics_an_opening_an_instrumental_and_credits() {
        let s = direct(&[], None, &track(200_000));
        assert_eq!(kinds(&s), [Kind::Opening, Kind::Instrumental, Kind::Credits]);
        assert_contiguous(&s);
        let blank = direct(&[line(1000, 2000, "  "), line(3000, 0, "♪")], None, &track(200_000));
        assert_eq!(kinds(&blank), kinds(&s));
    }

    #[test]
    fn a_song_without_gaps_has_no_instrumental() {
        let lines: Vec<Line> =
            (0..30).map(|i| line(5000 + i * 3000, 5000 + i * 3000 + 2800, &format!("line {i}"))).collect();
        let s = direct(&lines, None, &track(100_000));
        assert!(!kinds(&s).contains(&Kind::Instrumental), "{:?}", kinds(&s));
        assert_eq!(kinds(&s).first(), Some(&Kind::Opening));
        assert_eq!(kinds(&s).last(), Some(&Kind::Credits));
        assert_contiguous(&s);
    }

    #[test]
    fn a_single_line() {
        let s = direct(&[line(500, 0, "only this")], None, &track(30_000));
        assert_eq!(s.sections.len(), 2, "{:?}", kinds(&s));
        assert!(matches!(s.sections[0].kind, Kind::Close | Kind::Editorial));
        assert_eq!(s.sections[1].kind, Kind::Credits);
        assert_eq!(s.sections[0].lines, [0]);
        assert_eq!(s.ends, [4500]);
        assert_contiguous(&s);
    }

    #[test]
    fn sections_cover_the_whole_track() {
        for (lines, f, d) in [
            (song(), None, 60_000),
            (song(), Some(facts(60.0)), 60_000),
            (song(), Some(facts(60.0)), 0),
            (song(), None, 30_000),
            (vec![line(0, 100, "a"), line(100, 50, "a"), line(100, 0, "b")], None, 1000),
        ] {
            let s = direct(&lines, f.as_ref(), &track(d));
            assert_contiguous(&s);
            let shown: Vec<usize> = s.sections.iter().flat_map(|x| x.lines.clone()).collect();
            let sung = lines.iter().filter(|l| !norm(&l.text).is_empty()).count();
            assert_eq!(shown.len(), sung, "every sung line once: {:?}", s.sections);
        }
    }

    #[test]
    fn the_same_seed_writes_the_same_script() {
        let a = direct(&song(), Some(&facts(60.0)), &track(60_000));
        let b = direct(&song(), Some(&facts(60.0)), &track(60_000));
        assert_eq!(a, b);
        let aligns = |seed: &'static str| {
            let s = direct(&song(), None, &Track { seed, ..track(60_000) });
            s.sections.iter().map(|x| x.align).collect::<Vec<_>>()
        };
        assert_eq!(aligns("abcdefghijk"), aligns("abcdefghijk"));
    }
}
