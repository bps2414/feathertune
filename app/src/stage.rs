//! Stage (plugin `stage`): a directed full-screen cinema show over the synced lyrics. Rust writes the
//! script once per track (`direct`, pure and tested here) and builds a smooth Gaussian-blurred atmospheric
//! backdrop from the cover art (`artwork::stage_backdrop`); the `stage` page module (`injections/stage/`)
//! only plays it, in a fixed layer over the site, on the compositor.
//!
//! - The director reads the lines (word times, ends), the track analysis' facts when the track was
//!   measured (tempo, first beat, the loudness envelope), the title, artist, album, the next track and a
//!   seed (the video id), and writes sections: an opening up to the first line, blocks of lines (a gap
//!   over `max(3 s, 7 beats)`, 4.5 s without a tempo, or a change between repeated and unrepeated lines
//!   starts a new one), each a chorus (half its lines sung elsewhere too), a close-up (energy under 30%),
//!   an editorial verse (over 55%) or the one the last verse was not; an instrumental where the gap after
//!   a block is over `max(5 s, 8 beats)`; credits after the last line. A cut moves to a bar line within
//!   0.75 beat only between the end of the line before it and its first line's lead, so no word comes late
//!   and none is cut short. Each scene says how it is entered (`Entry`): a lift into a chorus, a breath out
//!   of one, a frame from the lines, a return to them. The energy is the low band × 0.6 plus the mid × 0.4,
//!   between the song's own 10th and 90th percentiles; it is also sent at 10 Hz (`light`) for the page's
//!   atmosphere and relief. Without facts the lyrics alone decide; without lyrics it is an opening, an
//!   instrumental and the credits.
//! - Opened from the title bar button or the tray: the window goes fullscreen (`titlebar::set_fullscreen`),
//!   the focus goes to the page and the page gets `{ open: true, script, lines, artwork, backdrop, halo, ... }`
//!   through `injections::send`. Closed by Esc (`titlebar.rs`, WebView2's `AcceleratorKeyPressed`), by the
//!   page's own fullscreen ending (the HUD's leave button enters and leaves it, so the existing
//!   `ContainsFullScreenElementChanged` path restores the window), by hiding or minimizing the window,
//!   by game mode and by the plugin's switch. The page asks Rust for nothing: no new command or permission.
//! - Each song gets a cohesive art direction (`Mood`) from its tempo, loudness, word pace and seed:
//!   `Cinema` (Instrument Serif + Plus Jakarta Sans), `Editorial` (Plus Jakarta Sans + Serif italic accents),
//!   `Poster` (Anybody condensed display in choruses + clean Sans verses), or `Nocturne` (Syne display +
//!   refined Sans/Serif contrast). Different sections within the same song pair distinct typographic roles
//!   so verses feel intimate and choruses feel monumental without repeating one monotonous style.
//! - Each line gets a shot (`Shot`) from 8 refined compositions (`Focus`, `Editorial`, `Hero`, `Split`,
//!   `Anthem`, `Climb`, `Echo`, `Crescendo`), a composed anchor (rule-of-thirds / center balance rather than
//!   random corner hopping), an emphasis word (longest held, favoring long and repeated words), a fluid
//!   entrance (`Illuminate`, `Curtain`, `Glide`, `Focus`, `Impact`) and a clean exit (`DriftUp`, `Dissolve`,
//!   `CurtainDown`, `Recede`). The chorus keeps a recognizable signature that escalates gently across
//!   repeats (level 1 to 3). A line continuing the previous sentence (`cont`) glides the previous line up.
//! - A duet (lines from both singers, TTML agents): each singer holds a side and the group the center.
//! - The bundled display fonts — Instrument Serif (regular & italic), Plus Jakarta Sans (variable weight),
//!   Syne (variable weight) and Anybody (variable width & weight), all under the SIL Open Font License
//!   (`injections/stage/OFL.txt`) — are compiled in and sent once as bytes (`stageFont`); the page loads
//!   them with the FontFace API.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use base64::Engine;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;

use crate::artwork;
use crate::audio_facts::Facts;
use crate::i18n::t;
use crate::lyrics::Line;
use crate::player::{Event, Player};
use crate::settings::Live;

/// Bundled Latin WOFF2 subsets (Fontsource, SIL OFL): family, style, weight range, optional stretch, bytes.
struct BundledFont {
    family: &'static str,
    style: &'static str,
    weight: &'static str,
    stretch: Option<&'static str>,
    bytes: &'static [u8],
}

const FONTS: [BundledFont; 5] = [
    BundledFont {
        family: "ft-serif",
        style: "normal",
        weight: "400",
        stretch: None,
        bytes: include_bytes!("../injections/stage/instrument-serif-latin-400-normal.woff2"),
    },
    BundledFont {
        family: "ft-serif",
        style: "italic",
        weight: "400",
        stretch: None,
        bytes: include_bytes!("../injections/stage/instrument-serif-latin-400-italic.woff2"),
    },
    BundledFont {
        family: "ft-sans",
        style: "normal",
        weight: "200 800",
        stretch: None,
        bytes: include_bytes!("../injections/stage/plus-jakarta-sans-latin-wght.woff2"),
    },
    BundledFont {
        family: "ft-syne",
        style: "normal",
        weight: "400 800",
        stretch: None,
        bytes: include_bytes!("../injections/stage/syne-latin-wght.woff2"),
    },
    BundledFont {
        family: "ft-poster",
        style: "normal",
        weight: "100 900",
        stretch: Some("50% 150%"),
        bytes: include_bytes!("../injections/stage/anybody-latin-wdth-wght.woff2"),
    },
];

/// Stage backdrop texture parameters (128×128 clamped Gaussian blur: smooth on 4K, ~8 KB PNG).
const BACKDROP_SIDE: u32 = 128;
const BACKDROP_SIGMA: f32 = 10.5;
/// Soft cover halo behind opening / instrumental artwork.
const HALO_SIDE: u32 = 96;
const HALO_SIGMA: f32 = 4.2;
const HALO_SATURATION: f32 = 1.6;
const HALO_BRIGHTNESS: f32 = 0.85;
const HALO_SOLID: f32 = 0.42;

/// A cut moves to a bar line at most this share of a beat away.
const SNAP_BEATS: f64 = 0.75;
/// A section starts this long before its first line.
const LEAD_MS: f64 = 150.0;
/// A line that comes this soon (ms) after the one before it ends may go on with its sentence.
const CONT_GAP_MS: f64 = 450.0;
/// Under these an opening or the credits are not worth a scene of their own.
const MIN_OPENING_MS: f64 = 1500.0;
const MIN_CREDITS_MS: f64 = 3000.0;
/// At most 15 minutes of light at 10 Hz (the envelope's own limit).
const MAX_LIGHT: usize = 9000;
/// Maximum chorus escalation level.
const MAX_LEVEL: u8 = 3;

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

/// The song's cohesive art direction; maps to a typographic pairing across sections on the page.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    /// Editorial cinema: Instrument Serif (with italic accents) + clean Plus Jakarta Sans.
    Cinema,
    /// Modern editorial: crisp Plus Jakarta Sans + Instrument Serif italic emphasis.
    Editorial,
    /// Sculptural poster: bold condensed Anybody in choruses/peaks + clean Sans verses.
    Poster,
    /// Nocturnal art-house: expressive Syne display + refined Sans/Serif contrast.
    Nocturne,
}

/// How a line is composed on screen (see `injections/stage/page.js`, `LAYOUT`).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Balanced cinema center stack.
    Focus,
    /// Asymmetric rule-of-thirds editorial column.
    Editorial,
    /// One sculptural emphasis word framed by refined supporting phrases.
    Hero,
    /// Two balanced phrases in counterweight separated by a hairline divider.
    Split,
    /// Chorus signature: commanding centered presence scaling with chorus level.
    Anthem,
    /// Two rows rising gently over the line's time.
    Climb,
    /// A word sung 2–4 times stacked with clean visual hierarchy.
    Echo,
    /// Climax: quiet build-up words leading into a commanding final word.
    Crescendo,
}

impl Layout {
    /// Whether a line of `words` words can take this layout.
    fn fits(self, words: usize) -> bool {
        match self {
            Layout::Hero | Layout::Split => words >= 2,
            Layout::Crescendo => (3..=8).contains(&words),
            Layout::Climb => (4..=12).contains(&words),
            Layout::Echo => (2..=12).contains(&words),
            Layout::Focus | Layout::Editorial | Layout::Anthem => words >= 1,
        }
    }
}

/// Verse layouts with their weights in a calm (`Close`) vs. driven (`Editorial`) scene.
const VERSE: [(Layout, u32, u32); 6] = [
    (Layout::Focus, 4, 2),
    (Layout::Editorial, 3, 3),
    (Layout::Hero, 2, 2),
    (Layout::Split, 1, 3),
    (Layout::Climb, 2, 2),
    (Layout::Crescendo, 1, 2),
];
/// Chorus signature partners following the `Anthem`.
const PARTNERS: [Layout; 4] = [Layout::Hero, Layout::Split, Layout::Crescendo, Layout::Editorial];
/// Fallback layouts that fit almost any line.
const FALLBACK: [Layout; 2] = [Layout::Focus, Layout::Editorial];

/// How a line's words enter: always smooth, never dry/blinking.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Enter {
    /// Whole line appears softly dimmed; each word illuminates smoothly as sung.
    Illuminate,
    /// Words rise cleanly out of an invisible baseline mask.
    Curtain,
    /// Gentle horizontal glide from the aligned side (or singer's side in a duet).
    Glide,
    /// Subtle scale-and-lift settle into crisp focus.
    Focus,
    /// Commanding yet smooth settle for choruses and climaxes.
    Impact,
}

/// How a line leaves: always smooth and collision-free.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
    DriftUp,
    Dissolve,
    CurtainDown,
    Recede,
}

/// How a scene is entered; the page plays it on the atmosphere and camera.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Entry {
    /// Camera blends smoothly into the new move.
    Cut,
    /// Into a chorus: gentle wind-up over the bar before its downbeat, then luminous open.
    Lift,
    /// Out of a chorus: calm pull back and atmospheric settle.
    Breath,
    /// From lines into a visual frame (instrumental, credits).
    Frame,
    /// From a visual frame back into lyrics.
    Return,
}

/// Subtle cinema camera drift over a scene.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Camera {
    Push,
    Pull,
    Left,
    Right,
    Rise,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
pub struct Shot {
    pub layout: Layout,
    /// Horizontal anchor in the safe area: -1 (left), 0 (center), 1 (right).
    pub x: i8,
    /// Vertical bias: -1 (upper), 0 (center), 1 (lower).
    pub y: i8,
    /// The emphasis word (an index into the line's words), if it has words.
    pub hero: Option<usize>,
    pub enter: Enter,
    pub exit: Exit,
    /// 0 outside the chorus; 1, 2, 3 for its first, second, third and later times.
    pub level: u8,
    /// Render the emphasis word in contrasting editorial italic serif.
    pub italic: bool,
    /// Who sings it in a duet: 0 the first singer, 1 the second, 2 together (always 0 outside a duet).
    pub singer: u8,
    /// A duet's answer: the other singer than the line before.
    pub answer: bool,
    /// Continues the sentence of the line before: the page glides that line up softly instead of exiting.
    pub cont: bool,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Section {
    pub kind: Kind,
    pub from_ms: u32,
    pub to_ms: u32,
    /// The lines it shows (indices into the lyrics' lines); empty outside the text scenes.
    pub lines: Vec<usize>,
    /// One per line, in the same order.
    pub shots: Vec<Shot>,
    pub camera: Camera,
    pub entry: Entry,
    /// 1 for the first chorus, 2 for the second...; 0 for anything else.
    pub chorus_no: u32,
    /// Why the director chose it, as a code (for diagnostics and tests; not shown).
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
    pub mood: Mood,
    /// Lines from both singers (see `Shot::singer`).
    pub duet: bool,
    pub grid: Option<Grid>,
    /// The energy at 10 Hz, 0 to 255 over the song's own range; empty without facts.
    pub light: Vec<u8>,
    /// Per line: how many times its text was sung before.
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

/// The envelope's energy, per 100 ms bin, 0 to 1, and the song's mean level before normalization.
struct Energy(Vec<f64>, Option<f64>);

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
            return Energy(Vec::new(), None);
        }
        let loud = level.iter().sum::<f64>() / level.len() as f64;
        let mut sorted = level.clone();
        sorted.sort_by(f64::total_cmp);
        let (p10, p90) = (sorted[sorted.len() / 10], sorted[sorted.len() * 9 / 10]);
        let span = if p90 - p10 > 1e-6 { p90 - p10 } else { 1.0 };
        Energy(level.iter().map(|x| ((x - p10) / span).clamp(0.0, 1.0)).collect(), Some(loud))
    }

    fn between(&self, a: f64, z: f64) -> Option<f64> {
        let from = (a.max(0.0) / 100.0).floor() as usize;
        let to = ((z / 100.0).ceil().max(0.0) as usize).min(self.0.len());
        (from < to).then(|| self.0[from..to].iter().sum::<f64>() / (to - from) as f64)
    }

    fn light(&self) -> Vec<u8> {
        self.0.iter().take(MAX_LIGHT).map(|e| (e * 255.0).round() as u8).collect()
    }
}

/// Seeded LCG: same seed always writes the same script.
struct Rng(u32);

impl Rng {
    fn new(seed: &str) -> Rng {
        Rng(seed.bytes().fold(7u32, |a, c| a.wrapping_mul(31).wrapping_add(u32::from(c))))
    }

    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        self.0
    }

    fn coin(&mut self) -> bool {
        self.next() >> 31 == 1
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 8) as usize % n.max(1)
    }

    fn weighted<T: Copy>(&mut self, items: &[(T, u32)]) -> Option<T> {
        let total: u32 = items.iter().map(|i| i.1).sum();
        if total == 0 {
            return None;
        }
        let mut at = self.below(total as usize) as u32;
        for &(item, weight) in items {
            if at < weight {
                return Some(item);
            }
            at -= weight;
        }
        None
    }
}

fn section(kind: Kind, from: f64, to: f64, why: &'static str) -> Section {
    Section {
        kind,
        from_ms: from.max(0.0).round() as u32,
        to_ms: to.max(0.0).round() as u32,
        lines: Vec::new(),
        shots: Vec::new(),
        camera: Camera::Push,
        entry: Entry::Cut,
        chorus_no: 0,
        why,
    }
}

fn letters(word: &str) -> usize {
    word.chars().filter(|c| c.is_alphanumeric()).count()
}

/// The words of a line with their times (ms); a line timed as a whole gets them spread over 85% of its
/// length by their letters, as the page does.
fn words(line: &Line, end: f64) -> Vec<(f64, String)> {
    if !line.words.is_empty() {
        return line
            .words
            .iter()
            .filter_map(|(ms, w)| {
                let text = w.trim();
                (!text.is_empty()).then(|| (f64::from(*ms), text.to_owned()))
            })
            .collect();
    }
    let parts: Vec<&str> = line.text.split_whitespace().collect();
    let ms = f64::from(line.ms);
    let span = (end - ms).max(300.0) * 0.85;
    let total = parts.iter().map(|p| p.chars().count()).sum::<usize>().max(1) as f64;
    let mut at = ms;
    parts
        .iter()
        .map(|p| {
            let w = (at, (*p).to_owned());
            at += span * p.chars().count() as f64 / total;
            w
        })
        .collect()
}

/// The line's emphasis word: the longest held, favoring long words and words the song sings repeatedly.
fn hero(words: &[(f64, String)], end: f64, counts: &HashMap<String, u32>) -> Option<usize> {
    let short = words.iter().all(|w| letters(&w.1) < 3);
    let score = |k: usize| {
        let held = (words.get(k + 1).map_or(end, |n| n.0) - words[k].0).max(1.0);
        let n = letters(&words[k].1);
        let sung = counts.get(&norm(&words[k].1)).copied().unwrap_or(0);
        held * (1.0 + 0.12 * n.min(10) as f64) * if sung >= 3 && n >= 4 { 1.4 } else { 1.0 }
    };
    (0..words.len()).filter(|&k| short || letters(&words[k].1) >= 3).max_by(|&a, &b| score(a).total_cmp(&score(b)))
}

/// Whether a line sings one of its words (two letters or more) two to four times, in twelve words or fewer.
fn echoes_a_word(words: &[(f64, String)]) -> bool {
    if words.len() > 12 {
        return false;
    }
    let mut count = HashMap::<String, u8>::new();
    for w in words {
        let n = norm(&w.1);
        if n.chars().count() >= 2 {
            *count.entry(n).or_default() += 1;
        }
    }
    count.values().any(|c| (2..=4).contains(c))
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end().ends_with(['.', '!', '?', '…'])
}

/// Whether line `b` continues the sentence of line `a` (which stops at `end`).
fn continues(a: &Line, b: &Line, end: f64) -> bool {
    let text = a.text.trim_end();
    if f64::from(b.ms) - end > CONT_GAP_MS || ends_sentence(text) || text.ends_with([':', ';']) {
        return false;
    }
    text.ends_with(',')
        || text.split_whitespace().count() <= 3
        || b.text.trim_start().chars().next().is_some_and(char::is_lowercase)
}

/// Layouts that feel cohesive within a continued sentence.
fn family(l: Layout) -> u8 {
    match l {
        Layout::Focus | Layout::Editorial | Layout::Climb | Layout::Echo => 0,
        Layout::Hero | Layout::Crescendo => 1,
        Layout::Split | Layout::Anthem => 2,
    }
}

/// Picks the song's art direction from tempo, loudness, word rate and seed so songs genuinely look
/// distinct instead of collapsing onto one display face.
fn mood(bpm: Option<f64>, loud: Option<f64>, words_per_s: Option<f64>, rng: &mut Rng) -> Mood {
    let parts = [bpm.map(|b| (b - 80.0) / 60.0), loud.map(|l| (l - 0.55) / 0.2), words_per_s.map(|w| (w - 1.5) / 2.0)];
    let known: Vec<f64> = parts.iter().flatten().map(|x| x.clamp(0.0, 1.0)).collect();
    let drive = if known.is_empty() { 0.5 } else { known.iter().sum::<f64>() / known.len() as f64 };
    if drive <= 0.32 {
        Mood::Cinema
    } else if drive >= 0.72 {
        Mood::Poster
    } else if drive >= 0.58 {
        if rng.coin() { Mood::Poster } else { Mood::Nocturne }
    } else {
        const MID: [Mood; 3] = [Mood::Cinema, Mood::Editorial, Mood::Nocturne];
        MID[rng.below(MID.len())]
    }
}

/// Gives each line of the text scenes its shot and each scene its camera move.
#[allow(clippy::too_many_arguments)]
fn shoot(
    sections: &mut [Section],
    lines: &[Line],
    ends: &[f64],
    energy: &Energy,
    counts: &HashMap<String, u32>,
    duet: bool,
    rng: &mut Rng,
) {
    // Chorus signature: Anthem centered, followed by two balanced partners across repeats.
    let first = rng.below(PARTNERS.len());
    let second = (first + 1 + rng.below(PARTNERS.len() - 1)) % PARTNERS.len();
    let sig = [Layout::Anthem, PARTNERS[first], PARTNERS[second]];
    let side_a: i8 = if rng.coin() { -1 } else { 1 };
    let places: [(i8, i8); 3] = [(0, 0), (side_a, 0), (-side_a, 0)];
    let enters = [Enter::Impact, Enter::Curtain, if rng.coin() { Enter::Focus } else { Enter::Glide }];
    let exits = [Exit::Recede, Exit::Dissolve, Exit::DriftUp];

    let mut order: Vec<(usize, usize, bool)> = Vec::new();
    let mut gap = true;
    for (si, s) in sections.iter().enumerate() {
        if s.lines.is_empty() {
            gap = true;
        }
        for j in 0..s.lines.len() {
            order.push((si, j, gap));
            gap = false;
        }
    }

    let mut prev: Option<Shot> = None;
    let (mut run, mut k) = (0u8, 0usize);
    let mut all: Vec<Vec<Shot>> = sections.iter().map(|s| Vec::with_capacity(s.lines.len())).collect();
    // Composed scene alignment: stays grounded instead of bouncing randomly around all 9 corners.
    let mut editorial_side: i8 = if rng.coin() { -1 } else { 1 };

    for &(si, j, after_gap) in &order {
        if after_gap {
            prev = None;
        }
        let s = &sections[si];
        let chorus = s.kind == Kind::Chorus;
        if chorus && j == 0 && (si == 0 || sections[si - 1].kind != Kind::Chorus) {
            run = run.saturating_add(1);
            k = 0;
        }
        let i = s.lines[j];
        let ws = words(&lines[i], ends[i]);
        let hero = hero(&ws, ends[i], counts);
        let hero_len = hero.map_or(0, |h| letters(&ws[h].1));
        let fits = |l: Layout| l.fits(ws.len());
        let before = prev.map(|p| p.layout);
        let singer = if duet { lines[i].voice.min(2) } else { 0 };
        let side: i8 = match singer {
            0 => -1,
            1 => 1,
            _ => 0,
        };
        let answer = duet && singer < 2 && prev.is_some_and(|p| p.singer < 2 && p.singer != singer);

        let mut shot = if chorus {
            let at = k % sig.len();
            let next = sig[(k + 1) % sig.len()];
            let layout = if fits(sig[at]) {
                sig[at]
            } else {
                FALLBACK.into_iter().find(|l| Some(*l) != before && *l != next).unwrap_or(Layout::Focus)
            };
            k += 1;
            let (x, y) = if duet {
                (side, 0)
            } else if matches!(layout, Layout::Anthem | Layout::Focus | Layout::Split) {
                (0, 0)
            } else {
                places[at]
            };
            Shot {
                layout,
                x,
                y,
                hero,
                enter: enters[at],
                exit: exits[at],
                level: run.min(MAX_LEVEL),
                italic: false,
                singer,
                answer,
                cont: false,
            }
        } else {
            let calm = s.kind == Kind::Close;
            let before_line = if after_gap { None } else { j.checked_sub(1).map(|p| s.lines[p]) };
            let cont = !duet
                && !prev.is_some_and(|p| p.cont)
                && before_line.is_some_and(|p| continues(&lines[p], &lines[i], ends[p]));
            let n_words = ws.len();
            let text = lines[i].text.trim_end();
            let kin = if cont { prev.map(|p| family(p.layout)) } else { None };
            let favour = |l: Layout| -> u32 {
                let mut w = 1;
                if kin.is_some_and(|f| family(l) == f) {
                    w *= 5;
                }
                if n_words <= 4 && matches!(l, Layout::Hero | Layout::Focus) {
                    w *= 3;
                }
                if n_words >= 8 && matches!(l, Layout::Editorial | Layout::Climb | Layout::Focus) {
                    w *= 3;
                }
                if text.ends_with(['!', '?']) && l == Layout::Crescendo {
                    w *= 4;
                }
                w
            };
            let mut pool: Vec<(Layout, u32)> = VERSE
                .iter()
                .filter(|v| fits(v.0) && Some(v.0) != before)
                .map(|&(l, quiet, driven)| (l, (if calm { quiet } else { driven }) * favour(l)))
                .collect();
            if before != Some(Layout::Echo) && fits(Layout::Echo) && echoes_a_word(&ws) {
                pool.push((Layout::Echo, 8 * favour(Layout::Echo)));
            }
            let layout = rng.weighted(&pool).or_else(|| pool.first().map(|p| p.0)).unwrap_or(Layout::Focus);

            // Composed framing: flip editorial side only on new sentences or new sections, never erratically.
            if before_line.is_none() || before_line.is_some_and(|p| ends_sentence(&lines[p].text)) {
                editorial_side = -editorial_side;
            }
            let (x, y) = if duet {
                (side, 0)
            } else {
                match layout {
                    Layout::Editorial => (editorial_side, 0),
                    Layout::Hero | Layout::Crescendo => (if calm { 0 } else { editorial_side }, 0),
                    Layout::Focus | Layout::Split | Layout::Climb | Layout::Echo | Layout::Anthem => (0, 0),
                }
            };

            let e = energy.between(f64::from(lines[i].ms), ends[i]).unwrap_or(if calm { 0.35 } else { 0.62 });
            let pick_enter: [(Enter, u32); 5] = if calm || e < 0.4 {
                [(Enter::Illuminate, 5), (Enter::Curtain, 3), (Enter::Focus, 2), (Enter::Glide, 1), (Enter::Impact, 0)]
            } else if e > 0.65 {
                [(Enter::Curtain, 3), (Enter::Glide, 3), (Enter::Focus, 2), (Enter::Impact, 2), (Enter::Illuminate, 1)]
            } else {
                [(Enter::Illuminate, 3), (Enter::Curtain, 3), (Enter::Glide, 2), (Enter::Focus, 2), (Enter::Impact, 0)]
            };
            let enters_pool: Vec<(Enter, u32)> =
                pick_enter.into_iter().filter(|x| x.1 > 0 && Some(x.0) != prev.map(|p| p.enter)).collect();

            let last_of_block = j + 1 == s.lines.len();
            let pick_exit: [(Exit, u32); 4] = [
                (Exit::DriftUp, 3),
                (Exit::Dissolve, 3),
                (Exit::CurtainDown, 2),
                (Exit::Recede, if last_of_block { 4 } else { 1 }),
            ];
            let exits_pool: Vec<(Exit, u32)> =
                pick_exit.into_iter().filter(|x| Some(x.0) != prev.map(|p| p.exit)).collect();

            Shot {
                layout,
                x,
                y,
                hero,
                enter: rng.weighted(&enters_pool).unwrap_or(Enter::Illuminate),
                exit: rng.weighted(&exits_pool).unwrap_or(Exit::Dissolve),
                level: 0,
                italic: false,
                singer,
                answer,
                cont,
            }
        };

        // Editorial italic contrast on emphasis words: restrained, never monotonous.
        shot.italic = match shot.layout {
            Layout::Hero => (3..=14).contains(&hero_len) && !prev.is_some_and(|p| p.italic),
            Layout::Focus | Layout::Editorial => {
                (4..=12).contains(&hero_len) && !prev.is_some_and(|p| p.italic) && rng.coin()
            }
            _ => false,
        };
        if answer {
            shot.enter = Enter::Glide;
        } else if duet && singer == 2 {
            shot.enter = Enter::Focus;
        }
        all[si].push(shot);
        prev = Some(shot);
    }

    let mut last = Camera::Push;
    for (s, shots) in sections.iter_mut().zip(all) {
        s.shots = shots;
        s.camera = match s.kind {
            Kind::Chorus | Kind::Opening => Camera::Push,
            Kind::Credits => Camera::Rise,
            _ => {
                let moves = [Camera::Pull, Camera::Left, Camera::Right, Camera::Rise, Camera::Push];
                let free: Vec<Camera> = moves.into_iter().filter(|m| *m != last).collect();
                free[rng.below(free.len())]
            }
        };
        last = s.camera;
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
    let offset = facts.and_then(|f| f.beat_offset_ms).filter(|o| o.is_finite()).unwrap_or(0.0);
    let snap = |t: f64, lo: f64, hi: f64| match beat {
        Some(beat) => {
            let bar = beat * 4.0;
            let k = ((t - offset) / bar).floor();
            [k, k + 1.0]
                .into_iter()
                .map(|k| offset + k * bar)
                .filter(|s| *s >= lo.max(0.0) && *s <= hi && (s - t).abs() < beat * SNAP_BEATS)
                .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
                .unwrap_or(t)
        }
        None => t,
    };
    let energy = Energy::of(facts);
    let ends: Vec<f64> = (0..lines.len()).map(|i| line_end(lines, i, duration)).collect();
    let texts: Vec<String> = lines.iter().map(|l| norm(&l.text)).collect();
    let sung: Vec<usize> = (0..lines.len()).filter(|&i| !texts[i].is_empty()).collect();
    let mut count = HashMap::<&str, u32>::new();
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
        let head = snap((duration * 0.2).min(12_000.0), 0.0, duration);
        let tail = snap(duration - (duration * 0.2).min(15_000.0), head, duration);
        out.push(section(Kind::Opening, 0.0, head, "no_lyrics"));
        out.push(section(Kind::Instrumental, head, tail, "no_lyrics"));
        out.push(section(Kind::Credits, tail, duration, "no_lyrics"));
    } else {
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
        let lead = |i: usize| f64::from(lines[i].ms) - LEAD_MS;
        let first = snap(lead(sung[0]), 0.0, lead(sung[0]));
        if first >= MIN_OPENING_MS {
            out.push(section(Kind::Opening, 0.0, first, "before_first"));
        }
        let mut last_verse = if rng.coin() { Kind::Editorial } else { Kind::Close };
        let mut chorus_no = 0;
        let instrumental = beat.map_or(5000.0, |b| (b * 8.0).max(5000.0));
        for (k, block) in blocks.iter().enumerate() {
            let end_of = |b: &[usize]| ends[*b.last().expect("a line")];
            let from = match k.checked_sub(1) {
                None => first.max(0.0),
                Some(p) => snap(lead(block[0]), end_of(&blocks[p]), lead(block[0])),
            };
            let end = end_of(block);
            let next = blocks.get(k + 1).map(|b| lead(b[0]));
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
            s.chorus_no = if chorus { chorus_no } else { 0 };
            match next {
                Some(next) if next + LEAD_MS - end > instrumental => {
                    let at = snap(end + beat.unwrap_or(500.0), end, next);
                    s.to_ms = at.round() as u32;
                    out.push(s);
                    out.push(section(Kind::Instrumental, at, snap(next, at, next), "gap"));
                }
                Some(next) => {
                    s.to_ms = snap(next, end, next).max(0.0).round() as u32;
                    out.push(s);
                }
                None => out.push(s),
            }
        }
        let last_end = ends[*sung.last().expect("a line")];
        let credits = snap(last_end + beat.unwrap_or(1000.0), last_end, duration);
        if duration - credits >= MIN_CREDITS_MS {
            if let Some(s) = out.last_mut() {
                s.to_ms = credits.round() as u32;
            }
            out.push(section(Kind::Credits, credits, duration, "after_last"));
        }
    }
    let duration_ms = duration.max(1.0).round() as u32;
    let mut sections = tidy(out, duration_ms);

    let mut counts = HashMap::<String, u32>::new();
    let (mut sung_words, mut sung_ms) = (0usize, 0.0);
    for &i in &sung {
        let ws = words(&lines[i], ends[i]);
        for w in &ws {
            *counts.entry(norm(&w.1)).or_default() += 1;
        }
        sung_words += ws.len();
        sung_ms += (ends[i] - f64::from(lines[i].ms)).clamp(0.0, 15_000.0);
    }
    let per_s = (sung_ms >= 5000.0).then(|| sung_words as f64 / (sung_ms / 1000.0));
    let mood = mood(bpm, energy.1, per_s, &mut rng);
    let duet = [0, 1].iter().all(|&v| sung.iter().filter(|&&i| lines[i].voice == v).count() >= 2);
    shoot(&mut sections, lines, &ends, &energy, &counts, duet, &mut rng);

    let text = |k: Kind| matches!(k, Kind::Close | Kind::Editorial | Kind::Chorus);
    for k in 1..sections.len() {
        let (was, is) = (sections[k - 1].kind, sections[k].kind);
        sections[k].entry = match (text(was), text(is)) {
            (false, true) => Entry::Return,
            (true, false) => Entry::Frame,
            (true, true) if is == Kind::Chorus && was != Kind::Chorus => Entry::Lift,
            (true, true) if was == Kind::Chorus && is != Kind::Chorus => Entry::Breath,
            _ => Entry::Cut,
        };
    }
    Script {
        duration_ms,
        sections,
        mood,
        duet,
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

/// Makes the sections contiguous from 0 to `duration`.
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

/// Upgrades Google/YouTube Music cover URLs that carry tiny dimensions (e.g. `=w120-h120`) to a crisp
/// 544×544 square while preserving the validated host and path.
fn hi_res_artwork(url: &str) -> Option<String> {
    let valid = crate::player::artwork(url)?;
    if let Some((base, suffix)) = valid.rsplit_once('=')
        && (valid.contains("googleusercontent.com/") || valid.contains("ggpht.com/"))
        && suffix.starts_with('w')
        && suffix.contains("-h")
        && !suffix.contains('/')
    {
        return Some(format!("{base}=w544-h544-l90-rj"));
    }
    Some(valid)
}

/// Cached backdrop + halo data URLs for the current cover, owned by Stage so it works even with
/// `ambient_mode` disabled.
#[derive(Default, Clone)]
struct StageArt {
    url: Option<String>,
    backdrop: Option<String>,
    halo: Option<String>,
}

static ART: Mutex<StageArt> = Mutex::new(StageArt { url: None, backdrop: None, halo: None });
static APP: OnceLock<AppHandle> = OnceLock::new();
/// The plugin was on at startup, so the page has the module.
static LOADED: AtomicBool = AtomicBool::new(false);
/// `LOADED` and the switch is still on.
static AVAILABLE: AtomicBool = AtomicBool::new(false);
static OPEN: AtomicBool = AtomicBool::new(false);
static FONT_SENT: AtomicBool = AtomicBool::new(false);

fn build_stage_art(url: &str) -> Result<(String, String), String> {
    let art = artwork::load(url)?;
    let encode = |pixels: windows::core::Result<Vec<u8>>, side: u32| {
        let png = artwork::png(&pixels.map_err(|e| e.to_string())?, side, side).map_err(|e| e.to_string())?;
        Ok::<_, String>(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png)))
    };
    let backdrop = encode(artwork::stage_backdrop(&art, BACKDROP_SIDE, BACKDROP_SIGMA), BACKDROP_SIDE)?;
    let halo =
        encode(artwork::halo(&art, HALO_SIDE, HALO_SIGMA, HALO_SATURATION, HALO_BRIGHTNESS, HALO_SOLID), HALO_SIDE)?;
    Ok((backdrop, halo))
}

pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    if !app.state::<Live>().plugin("stage") {
        return;
    }
    LOADED.store(true, Ordering::Relaxed);
    AVAILABLE.store(true, Ordering::Relaxed);

    // Track metadata & script refreshes.
    let mut rx = app.state::<Player>().subscribe();
    std::thread::spawn(move || {
        let mut last_meta: Option<(Option<String>, String, Option<String>, i64)> = None;
        loop {
            match rx.blocking_recv() {
                Ok(Event::Track(s)) => {
                    last_meta =
                        Some((s.track.id, s.track.album, s.track.artwork, (s.track.duration * 2.0).round() as i64));
                    refresh();
                }
                Ok(Event::Progress(s)) => {
                    let meta = (s.track.id, s.track.album, s.track.artwork, (s.track.duration * 2.0).round() as i64);
                    if last_meta.as_ref() != Some(&meta) {
                        last_meta = Some(meta);
                        if is_open() {
                            refresh();
                        }
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });

    // Dedicated cover art worker for Stage's smooth Gaussian backdrop and halo.
    let mut art_rx = app.state::<Player>().subscribe();
    std::thread::spawn(move || {
        let mut last_url: Option<Option<String>> = None;
        loop {
            match art_rx.blocking_recv() {
                Ok(event) => {
                    let wanted = event.state().track.artwork.clone();
                    if last_url.as_ref() == Some(&wanted) {
                        continue;
                    }
                    last_url = Some(wanted.clone());
                    let (backdrop, halo) = match wanted.as_deref().map(build_stage_art) {
                        Some(Ok((b, h))) => (Some(b), Some(h)),
                        Some(Err(e)) => {
                            eprintln!("stage: backdrop not built: {e}");
                            (None, None)
                        }
                        None => (None, None),
                    };
                    *ART.lock().unwrap_or_else(|e| e.into_inner()) = StageArt { url: wanted, backdrop, halo };
                    if is_open() {
                        refresh();
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });

    let mut queue_rx = app.state::<Player>().subscribe_queue();
    std::thread::spawn(move || {
        loop {
            match queue_rx.blocking_recv() {
                Ok(_) => {
                    if is_open() {
                        refresh();
                    }
                }
                Err(RecvError::Lagged(_)) => {}
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

/// The plugin's switch: off closes the stage and takes the button away until it is on again.
pub fn set_enabled(app: &AppHandle, on: bool) {
    if !on {
        close(app);
        FONT_SENT.store(false, Ordering::Relaxed);
        crate::injections::send(app, "stageFont", &Value::Null);
    }
    AVAILABLE.store(on && LOADED.load(Ordering::Relaxed), Ordering::Relaxed);
    crate::titlebar::refresh(app);
}

/// Opens the stage: the window comes back if hidden, goes fullscreen, the page gets focus and the script.
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
        let fonts: Vec<Value> = FONTS
            .iter()
            .map(|f| {
                let mut obj = json!({
                    "family": f.family,
                    "style": f.style,
                    "weight": f.weight,
                    "data": base64::engine::general_purpose::STANDARD.encode(f.bytes),
                });
                if let Some(s) = f.stretch {
                    obj["stretch"] = Value::String(s.into());
                }
                obj
            })
            .collect();
        crate::injections::send(app, "stageFont", &json!({ "fonts": fonts }));
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

/// Closes the stage on the page only (window minimized: leaves fullscreen on restore, see titlebar.rs).
pub fn close_page(app: &AppHandle) -> bool {
    if !OPEN.swap(false, Ordering::Relaxed) {
        return false;
    }
    crate::diag::log("stage", "closed");
    crate::injections::send(app, "stage", &json!({ "open": false }));
    true
}

/// Writes the script again (the track, its lines, its facts or its backdrop changed); only while open.
pub fn refresh() {
    if let Some(app) = APP.get().filter(|_| is_open()) {
        crate::injections::send(app, "stage", &payload(app));
    }
}

/// What the page gets while open (see `injections/stage/page.js`).
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
    let timed = |ws: &crate::lyrics::Words| -> Value {
        if ws.is_empty() { Value::Null } else { ws.iter().map(|(ms, w)| json!([ms, w])).collect() }
    };
    let page_lines: Vec<Value> =
        lines.iter().map(|l| json!([l.ms, l.text, timed(&l.words), l.voice, timed(&l.bg)])).collect();
    let subtitles = app.state::<Live>().get().stage.translation;
    let translation = (subtitles && lines.iter().any(|l| l.translation.is_some()))
        .then(|| lines.iter().map(|l| l.translation.as_deref()).collect::<Vec<_>>());
    let art = ART.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let art_matches = art.url == track.artwork;
    json!({
        "open": true,
        "id": track.id,
        "artwork": track.artwork.as_deref().and_then(hi_res_artwork),
        "backdrop": if art_matches { art.backdrop } else { None },
        "halo": if art_matches { art.halo } else { None },
        "script": script,
        "lines": page_lines,
        "translation": translation,
        "texts": {
            "leave": t("Leave the stage"),
            "esc": t("Esc to leave"),
            "play": t("Play"),
            "pause": t("Pause"),
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

    fn long_song() -> Vec<Line> {
        let chorus = ["Hold on to the night", "We are burning slow", "Hold on hold on", "Till the morning goes"];
        let verse = |n: u32| -> Vec<String> {
            (0..4).map(|k| format!("verse {n} line {k} with a few more words here and there")).collect()
        };
        let mut out = Vec::new();
        let mut at = 8000;
        let mut push = |text: &str, timed: bool, at: &mut u32| {
            let mut l = line(*at, *at + 2800, text);
            if timed {
                let parts: Vec<&str> = text.split(' ').collect();
                l.words = parts.iter().enumerate().map(|(k, w)| (*at + k as u32 * 500, format!("{w} "))).collect();
            }
            out.push(l);
            *at += 3000;
        };
        for n in 0..2 {
            for v in verse(n) {
                push(&v, false, &mut at);
            }
            for c in chorus {
                push(c, true, &mut at);
            }
        }
        push("a bridge that is sung once only", false, &mut at);
        push("and then a long breath", false, &mut at);
        at += 12_000;
        for c in chorus {
            push(c, true, &mut at);
        }
        out
    }

    fn shots(s: &Script) -> Vec<(Shot, bool, Kind)> {
        let mut out = Vec::new();
        let mut gap = true;
        for x in &s.sections {
            if x.lines.is_empty() {
                gap = true;
            }
            assert_eq!(x.shots.len(), x.lines.len(), "one shot per line");
            for shot in &x.shots {
                out.push((*shot, gap, x.kind));
                gap = false;
            }
        }
        out
    }

    #[test]
    fn repeated_lines_are_a_chorus() {
        let s = direct(&song(), None, &track(60_000));
        let choruses: Vec<&Section> = s.sections.iter().filter(|s| s.kind == Kind::Chorus).collect();
        assert_eq!(choruses.len(), 2, "{:?}", kinds(&s));
        assert_eq!((choruses[0].chorus_no, choruses[1].chorus_no), (1, 2));
        assert_eq!(choruses[0].lines, [2, 3]);
        assert_eq!(choruses[0].entry, Entry::Lift);
        assert_eq!(kinds(&s)[3], Kind::Instrumental);
        assert_eq!(
            s.sections.iter().map(|x| x.entry).skip(3).take(2).collect::<Vec<_>>(),
            [Entry::Frame, Entry::Return]
        );
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
    fn cuts_land_on_the_bar_without_cutting_a_line() {
        let lines = song();
        let s = direct(&lines, Some(&facts(60.0)), &track(60_000));
        assert_eq!(s.sections[0].kind, Kind::Opening);
        assert_eq!(s.sections[0].to_ms, 9950);
        let chorus = s.sections.iter().find(|x| x.kind == Kind::Chorus).expect("a chorus");
        assert_eq!(chorus.from_ms, 16_000);
        for x in s.sections.iter().filter(|x| !x.lines.is_empty()) {
            assert!(f64::from(x.from_ms) <= f64::from(lines[x.lines[0]].ms) - LEAD_MS, "{x:?}");
            assert!(x.lines[0].checked_sub(1).is_none_or(|p| x.from_ms >= lines[p].end), "{x:?}");
        }
        assert_eq!(s.grid, Some(Grid { bpm: 120.0, offset_ms: 0.0 }));
        assert_eq!(s.light.len(), 600);
        assert_contiguous(&s);
    }

    #[test]
    fn without_facts_the_lyrics_decide() {
        let s = direct(&song(), None, &track(60_000));
        assert!(s.grid.is_none() && s.light.is_empty());
        assert!(s.sections.iter().all(|x| !matches!(x.why, "low_energy" | "high_energy")));
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
    fn no_layout_twice_in_a_row_and_every_layout_fits() {
        let seeds = ["abcdefghijk", "dQw4w9WgXcQ", "zzzzzzzzzzz", "0", "kJQP7kiw5Fk", "x1y2z3", "seed"];
        for seed in seeds {
            for (lines, f) in [(song(), None), (long_song(), None), (long_song(), Some(facts(160.0)))] {
                let s = direct(&lines, f.as_ref(), &Track { seed, ..track(160_000) });
                let all = shots(&s);
                for w in all.windows(2) {
                    let ((a, _, _), (b, gap, _)) = (w[0], w[1]);
                    if !gap {
                        assert_ne!(a.layout, b.layout, "{seed}: {all:?}");
                    }
                }
                for x in &s.sections {
                    for (&i, shot) in x.lines.iter().zip(&x.shots) {
                        assert!(shot.layout != Layout::Anthem || x.kind == Kind::Chorus);
                        let n = lines[i].text.split_whitespace().count();
                        assert!(shot.layout.fits(n), "{:?} for {n} words", shot.layout);
                        assert!(shot.hero.is_some_and(|h| h < n));
                    }
                }
            }
        }
    }

    #[test]
    fn chorus_repeats_share_the_signature_and_escalate() {
        for seed in ["abcdefghijk", "dQw4w9WgXcQ", "zzzzzzzzzzz"] {
            let s = direct(&long_song(), None, &Track { seed, ..track(160_000) });
            let runs: Vec<Vec<Shot>> =
                s.sections.iter().filter(|x| x.kind == Kind::Chorus).map(|x| x.shots.clone()).collect();
            assert_eq!(runs.len(), 3, "{:?}", kinds(&s));
            let look = |r: &[Shot]| r.iter().map(|x| (x.layout, x.x, x.y, x.enter, x.exit)).collect::<Vec<_>>();
            assert_eq!(look(&runs[0]), look(&runs[1]));
            assert_eq!(look(&runs[0]), look(&runs[2]));
            assert_eq!(runs[0][0].layout, Layout::Anthem);
            assert_eq!((runs[0][0].x, runs[0][0].y), (0, 0));
            let levels: Vec<u8> = runs.iter().map(|r| r[0].level).collect();
            assert_eq!(levels, [1, 2, 3]);
            assert!(s.sections.iter().filter(|x| x.kind != Kind::Chorus).all(|x| x.shots.iter().all(|y| y.level == 0)));
        }
    }

    #[test]
    fn the_same_seed_writes_the_same_script() {
        let a = direct(&long_song(), Some(&facts(160.0)), &track(160_000));
        let b = direct(&long_song(), Some(&facts(160.0)), &track(160_000));
        assert_eq!(a, b);
    }

    fn duet_song() -> Vec<Line> {
        [0, 0, 1, 0, 1, 1, 2, 2, 0, 1, 0, 2]
            .iter()
            .enumerate()
            .map(|(k, &v)| {
                let at = 6000 + k as u32 * 3000;
                Line { voice: v, ..line(at, at + 2800, &format!("duet line number {k} sung here")) }
            })
            .collect()
    }

    #[test]
    fn a_duet_gives_each_singer_a_side_and_answers() {
        let lines = duet_song();
        for seed in ["abcdefghijk", "dQw4w9WgXcQ", "zzzzzzzzzzz"] {
            let s = direct(&lines, None, &Track { seed, ..track(60_000) });
            assert!(s.duet);
            let mut prev: Option<u8> = None;
            for x in &s.sections {
                if x.lines.is_empty() {
                    prev = None;
                }
                for (&i, shot) in x.lines.iter().zip(&x.shots) {
                    let v = lines[i].voice;
                    assert_eq!((shot.singer, shot.x), (v, [-1, 1, 0][usize::from(v)]), "{seed}: line {i}");
                    let answer = prev.is_some_and(|p| p < 2 && v < 2 && p != v);
                    assert_eq!(shot.answer, answer, "{seed}: line {i}");
                    assert!(!answer || shot.enter == Enter::Glide);
                    prev = Some(v);
                }
            }
        }
    }

    #[test]
    fn the_mood_follows_tempo_and_loudness_and_diversifies() {
        let with = |bpm: f64, level: u8| {
            let mut f = facts(160.0);
            f.bpm = Some(bpm);
            let bins = 1600;
            let bytes: Vec<u8> = (0..bins).flat_map(|b| [level.saturating_add((b % 7) as u8), level, 60]).collect();
            f.envelope = base64::engine::general_purpose::STANDARD.encode(bytes);
            direct(&long_song(), Some(&f), &track(160_000)).mood
        };
        assert_eq!(with(155.0, 240), Mood::Poster);
        assert_eq!(with(66.0, 85), Mood::Cinema);
        // Mid-tempo tracks diversify across seeds instead of collapsing to one face.
        let mid: std::collections::HashSet<u8> = ["a1", "b2", "c3", "d4", "e5", "f6", "g7", "h8"]
            .into_iter()
            .map(|seed| direct(&long_song(), None, &Track { seed, ..track(160_000) }).mood as u8)
            .collect();
        assert!(mid.len() >= 2, "expected multiple moods across seeds: {mid:?}");
    }

    #[test]
    fn hi_res_artwork_upgrades_tiny_google_thumbnails_and_rejects_bad_hosts() {
        assert_eq!(
            hi_res_artwork("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj").as_deref(),
            Some("https://lh3.googleusercontent.com/abc=w544-h544-l90-rj")
        );
        assert_eq!(
            hi_res_artwork("https://i.ytimg.com/vi/abc12345678/hqdefault.jpg").as_deref(),
            Some("https://i.ytimg.com/vi/abc12345678/hqdefault.jpg")
        );
        assert_eq!(hi_res_artwork("https://evil.example.com/a.jpg"), None);
    }
}
