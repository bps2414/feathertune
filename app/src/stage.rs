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
//!   a block is over `max(5 s, 8 beats)`; credits after the last line. A cut moves to a bar line within
//!   0.75 beat only between the end of the line before it and its first line's lead, so no word comes late
//!   and none is cut short. Each scene says how it is entered (`Entry`): a lift into a chorus, a breath out
//!   of one, a frame from the lines, a return to them. The energy is the low band × 0.6 plus the mid × 0.4,
//!   between the song's own 10th and 90th percentiles; it is also sent at 10 Hz (`light`) for the page's
//!   light and relief. Without
//!   facts the lyrics alone decide; without lyrics it is an opening, an instrumental and the credits.
//! - Opened from the title bar button or the tray: the window goes fullscreen (`titlebar::set_fullscreen`),
//!   the focus goes to the page and the page gets `{ open: true, script, lines, ... }` through
//!   `injections::send`. Closed by Esc (titlebar.rs, WebView2's `AcceleratorKeyPressed`), by the page's
//!   own fullscreen ending (the HUD's leave button enters and leaves it, so the existing
//!   `ContainsFullScreenElementChanged` path restores the window), by hiding or minimizing the window,
//!   by game mode and by the plugin's switch. The page asks Rust for nothing: no new command or permission.
//! - Each line gets a shot (`Shot`): a layout from a varied set (never the same twice in a row, nor the same
//!   place on screen, between two lines with no textless scene between them), where it sits, its emphasis
//!   word (the longest held, favouring long words and words the song repeats), how its words are born with
//!   the voice and how it leaves. The layout follows the line first (short lines go big, long ones run in
//!   rows, a word sung twice echoes, a cry bursts) and the seed breaks ties; the places walk a path through
//!   the scene, one neighbour a line, jumping only at a new sentence, a scene's start or a downbeat; a line
//!   that goes on with the sentence before it keeps that line's family and pushes it up (`Shot::cont`);
//!   what covers the screen on its way out only leaves with room before the next line. The set borrows
//!   from lyric edits: words piling up small until the last one bursts big (`Burst`), a few words down a
//!   diagonal with script ends (`Stair`), two rows rising (`Climb`), a repeated word stacked in a column
//!   (`Echo`), the emphasis word in a script face. The chorus has a signature: the anthem then two seeded
//!   partners, in the same order, entrances and places every time it comes back, one level higher each
//!   time (bigger, more glow, up to 3). The peak is the text in bars (`Bar`): on the anthem of the strongest
//!   choruses after the first (or of the only one), at most twice a song. The paper behind the lines
//!   (`Paper`) only cuts on an event: a scene's start (every other driven verse is on light paper), a bar,
//!   a duet's answer after 6 s on the same paper.
//! - A duet (lines from both singers, TTML agents): each singer has a side and the group the middle; a
//!   line from the other singer than the line before is an answer, its letters typed over the held note.
//! - Each scene gets a camera move. The song gets a type personality (`Voice`) from its tempo, loudness
//!   and how fast its words come (fast and loud: narrow and heavy; slow or quiet: wide and light; in
//!   between, the seed picks), so songs look different while one song stays coherent.
//! - The script is written again while open when the track, its lines, its facts or its background change.
//! - The display fonts, Anybody (width and weight axes), Unbounded and Syne (weight), and the script face,
//!   Great Vibes, all under the SIL Open Font License (`injections/stage/OFL.txt`), are compiled in and sent
//!   once as bytes (`stageFont`); the page loads them with the FontFace API.

use std::collections::HashMap;
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

/// The variable latin subsets (Fontsource's WOFF2): the page's family name and the file.
const FONTS: [(&str, &[u8]); 4] = [
    ("ft-anybody", include_bytes!("../injections/stage/anybody-latin-wdth-wght.woff2")),
    ("ft-unbounded", include_bytes!("../injections/stage/unbounded-latin-wght.woff2")),
    ("ft-syne", include_bytes!("../injections/stage/syne-latin-wght.woff2")),
    ("ft-script", include_bytes!("../injections/stage/great-vibes-latin-400.woff2")),
];
/// The text in bars is the peak: at most this many a song.
const MAX_BARS: usize = 2;
/// A duet's answer only turns the paper once the paper has stayed this long (ms).
const PAPER_HOLD_MS: f64 = 6000.0;
/// A cut moves to a bar line at most this share of a beat away.
const SNAP_BEATS: f64 = 0.75;
/// A section starts this long before its first line.
const LEAD_MS: f64 = 150.0;
/// A line that comes this soon (ms) after the one before it ends may go on with its sentence.
const CONT_GAP_MS: f64 = 400.0;
/// What covers the screen on its way out (a zoom, a scatter) needs this long (ms) before the next line.
const ROOM_MS: f64 = 600.0;
/// The paths a scene's lines walk through the 3×3 places, one step to a neighbour a line: the diagonals
/// through the middle, a zigzag, around the edge and into the middle.
const PATHS: [&[(i8, i8)]; 4] = [
    &[(-1, -1), (0, 0), (1, 1), (0, 0)],
    &[(1, -1), (0, 0), (-1, 1), (0, 0)],
    &[(-1, -1), (0, 0), (1, -1), (1, 0), (0, 1), (-1, 0)],
    &[(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (0, 0)],
];
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

/// The type's personality for the whole song; the page maps it to a face and its axes.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Voice {
    /// Anybody, narrow and heavy, in capitals: fast and loud songs.
    Compressed,
    /// Anybody, wide and light, as written: slow or quiet songs.
    Expanded,
    /// Unbounded, in lower case.
    Round,
    /// Syne, as written (it widens as it gets heavier).
    Quirk,
}

/// How a line is set on screen (see injections/stage/page.js, `LAYOUTS`).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Balanced rows, centered.
    Stack,
    /// Rows flush to one side.
    Rag,
    /// Rows stepping diagonally.
    Cascade,
    /// The emphasis word giant, the rest small around it.
    Hero,
    /// Each word in a cell of a grid.
    Grid,
    /// The emphasis word turned on its side along an edge, the rest beside it.
    Vertical,
    /// The two halves on two rows in two tones, apart.
    Split,
    /// A stack on a plane tilted in perspective.
    Tilt,
    /// The chorus' own: big, centered.
    Anthem,
    /// The words pile up small, the last one bursts big with a full stop.
    Burst,
    /// The rows in solid bars, full width, posing on the beat: the peak.
    Bar,
    /// Two to four words down a diagonal, the ends big in the script face, a star behind the first.
    Stair,
    /// Two rows rising over the line's time, a star rising with them.
    Climb,
    /// A word the line sings two to four times stacked in a column, a row from each time, each row a
    /// little bigger.
    Echo,
}

impl Layout {
    /// Whether a line of `words` words, its emphasis word `hero` letters long, can take it.
    fn fits(self, words: usize, hero: usize) -> bool {
        match self {
            Layout::Hero | Layout::Split => words >= 2,
            Layout::Cascade => words >= 3,
            Layout::Grid => (3..=8).contains(&words),
            Layout::Vertical => words >= 3 && hero <= 10,
            Layout::Burst => (3..=7).contains(&words),
            Layout::Bar => (1..=8).contains(&words),
            Layout::Stair => (2..=4).contains(&words),
            Layout::Climb => (4..=12).contains(&words),
            // And a word sung two to four times (`echoes_a_word`).
            Layout::Echo => (2..=12).contains(&words),
            _ => true,
        }
    }
}

/// The verse layouts with their weights in a close-up (calm) and in an editorial (driven) scene.
const VERSE: [(Layout, u32, u32); 11] = [
    (Layout::Stack, 2, 1),
    (Layout::Rag, 3, 2),
    (Layout::Hero, 2, 2),
    (Layout::Vertical, 2, 1),
    (Layout::Tilt, 1, 1),
    (Layout::Split, 1, 3),
    (Layout::Cascade, 1, 2),
    (Layout::Grid, 0, 2),
    (Layout::Burst, 1, 3),
    (Layout::Stair, 3, 2),
    (Layout::Climb, 2, 1),
];
/// The chorus' signature is the anthem and two of these.
const PARTNERS: [Layout; 4] = [Layout::Hero, Layout::Split, Layout::Burst, Layout::Climb];
/// A chorus line whose signature layout does not fit it takes the first of these that differs from its
/// neighbours (always fits).
const FALLBACK: [Layout; 3] = [Layout::Stack, Layout::Rag, Layout::Tilt];
/// The escalation stops here.
const MAX_LEVEL: u8 = 3;

/// How a line's words are born: each only when it is sung, never before.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Enter {
    /// Dry: there at once, a flash of glow, a short settle in size.
    Pop,
    /// Its letters typed over the time the word is held.
    Letters,
    /// A short slide in from the singer's side (from both sides for a group).
    Drift,
    /// Up from its baseline, inside a mask.
    Rise,
    /// The emphasis word lands big and settles with a small overshoot; the others pop.
    Stamp,
    /// Its two halves slide in from opposite sides and meet.
    Slice,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Exit {
    Slide,
    /// Each word flies off its own way.
    Scatter,
    /// The rows fold away on their top edge.
    Fold,
    /// Past the camera.
    Zoom,
    Sink,
    /// Off in three dry steps.
    Steps,
}

/// The paper behind the lines: the lit stage, or a cut to light paper with dark type.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Paper {
    Dark,
    Light,
}

/// How a scene is entered; the page plays it on the light and the camera (the lines have their own exits).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Entry {
    /// The camera's move only blends into the new one.
    Cut,
    /// Into a chorus: the bar before its first downbeat winds up (the camera closes in, the light gathers)
    /// and the downbeat opens it with a flash.
    Lift,
    /// Out of a chorus: a bar's pull back with the light down.
    Breath,
    /// From the lines to a scene without them: its picture comes in from where the last line was.
    Frame,
    /// From a scene without lines back to them: that scene leaves before the first word.
    Return,
}

/// The camera's slow move over a scene.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Camera {
    Push,
    Pull,
    Left,
    Right,
    Rise,
    Roll,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
pub struct Shot {
    pub layout: Layout,
    /// Where the block sits in the safe area: -1, 0 or 1 across and down (the sign of `x` is also the
    /// direction of the layouts that have one).
    pub x: i8,
    pub y: i8,
    /// The emphasis word (an index into the line's words), if it has words.
    pub hero: Option<usize>,
    pub enter: Enter,
    pub exit: Exit,
    /// 0 outside the chorus; 1, 2, 3 for its first, second, third and later times.
    pub level: u8,
    pub paper: Paper,
    /// The emphasis word in the script face.
    pub script: bool,
    /// Who sings it in a duet: 0 the first singer, 1 the second, 2 together (always 0 outside a duet).
    pub singer: u8,
    /// A duet's answer: the other singer than the line before.
    pub answer: bool,
    /// It goes on with the sentence of the line before (`continues`): the page pushes that one up, small,
    /// instead of letting it leave. Never two in a row.
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
    pub voice: Voice,
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

/// The envelope's energy (see the module doc), per 100 ms bin, 0 to 1, and the song's mean level before
/// it is spread over its own range (0 to 1 for -60 to 0 dBFS; None without facts).
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

/// The seeded choices (which verse look starts, the layouts, places and moves): the same seed, the same
/// script.
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

    /// 0 to `n - 1` (the high bits: the low ones of this generator repeat quickly).
    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 8) as usize % n.max(1)
    }

    /// One of `items` by weight; None when every weight is 0.
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
        return line.words.iter().map(|(ms, w)| (f64::from(*ms), w.trim().to_owned())).collect();
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

/// The line's emphasis word: the longest held, favouring long words and words the song sings at least three
/// times; words under three letters only when there is nothing else.
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

/// Whether a line sings one of its words (two letters or more) two to four times, in twelve words or fewer:
/// the echo's rows.
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

/// Whether line `b` goes on with the sentence of line `a`, which stops being sung at `end`: `b` comes at
/// once, `a` does not end a sentence, and `a` ends on a comma or is a fragment of three words or fewer, or
/// `b` starts in lower case.
fn continues(a: &Line, b: &Line, end: f64) -> bool {
    let text = a.text.trim_end();
    if f64::from(b.ms) - end > CONT_GAP_MS || ends_sentence(text) || text.ends_with([':', ';']) {
        return false;
    }
    text.ends_with(',')
        || text.split_whitespace().count() <= 3
        || b.text.trim_start().chars().next().is_some_and(char::is_lowercase)
}

/// The layouts that read alike: the rest of a sentence keeps to its line's family.
fn family(l: Layout) -> u8 {
    match l {
        Layout::Stack | Layout::Rag | Layout::Cascade | Layout::Climb | Layout::Echo => 0,
        Layout::Hero | Layout::Vertical | Layout::Burst | Layout::Stair => 1,
        _ => 2,
    }
}

/// The song's type personality: the tempo, the loudness and the words per second, each that is known,
/// as a drive from 0 (calm) to 1; at or over 0.62 compressed, at or under 0.3 expanded, the seed picks in
/// between.
fn voice(bpm: Option<f64>, loud: Option<f64>, words_per_s: Option<f64>, rng: &mut Rng) -> Voice {
    let parts = [bpm.map(|b| (b - 80.0) / 60.0), loud.map(|l| (l - 0.55) / 0.2), words_per_s.map(|w| (w - 1.5) / 2.0)];
    let known: Vec<f64> = parts.iter().flatten().map(|x| x.clamp(0.0, 1.0)).collect();
    let drive = if known.is_empty() { 0.5 } else { known.iter().sum::<f64>() / known.len() as f64 };
    let pick = rng.coin();
    if drive >= 0.62 {
        Voice::Compressed
    } else if drive <= 0.3 {
        Voice::Expanded
    } else if pick {
        Voice::Round
    } else {
        Voice::Quirk
    }
}

const PLACES: [(i8, i8); 9] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
const CENTER: (i8, i8) = (0, 0);

/// Gives each line of the text scenes its shot and each scene its camera (see the module doc). `grid` is
/// the beat and the first beat (ms) when the tempo is known.
#[allow(clippy::too_many_arguments)]
fn shoot(
    sections: &mut [Section],
    lines: &[Line],
    ends: &[f64],
    energy: &Energy,
    counts: &HashMap<String, u32>,
    duet: bool,
    grid: Option<(f64, f64)>,
    rng: &mut Rng,
) {
    // A line that starts right on a bar line.
    let on_bar = |ms: f64| {
        grid.is_some_and(|(beat, off)| {
            let ph = ((ms - off) / beat).rem_euclid(4.0);
            !(0.15..=3.85).contains(&ph)
        })
    };
    // The chorus' signature, once per song: the anthem in the middle, then two partners elsewhere.
    let first = rng.below(PARTNERS.len());
    let second = (first + 1 + rng.below(PARTNERS.len() - 1)) % PARTNERS.len();
    let sig = [Layout::Anthem, PARTNERS[first], PARTNERS[second]];
    let off: Vec<(i8, i8)> = PLACES.iter().copied().filter(|p| *p != CENTER).collect();
    let a = off[rng.below(off.len())];
    let rest: Vec<(i8, i8)> = off.iter().copied().filter(|p| *p != a).collect();
    let places = [CENTER, a, rest[rng.below(rest.len())]];
    let enters = [Enter::Pop, Enter::Drift, if rng.coin() { Enter::Letters } else { Enter::Rise }];
    let exits = [Exit::Zoom, if rng.coin() { Exit::Scatter } else { Exit::Fold }, Exit::Steps];

    // Every line of a text scene, in order, with whether a textless scene comes before it.
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
    // The verse's walk: which path, where on it, which way.
    let mut walk = (0usize, 0usize, true);
    let (mut run, mut k) = (0u8, 0usize);
    // Per line of `order`: its chorus run (1, 2... ; 0 outside one), its word count and its energy.
    let mut info: Vec<(usize, usize, Option<f64>)> = Vec::with_capacity(order.len());
    let mut runs = 0usize;
    let mut all: Vec<Vec<Shot>> = sections.iter().map(|s| Vec::with_capacity(s.lines.len())).collect();
    for (n, &(si, j, after_gap)) in order.iter().enumerate() {
        if after_gap {
            prev = None;
        }
        let s = &sections[si];
        let chorus = s.kind == Kind::Chorus;
        if chorus && j == 0 && (si == 0 || sections[si - 1].kind != Kind::Chorus) {
            run = run.saturating_add(1);
            runs += 1;
            k = 0;
        }
        let i = s.lines[j];
        let ws = words(&lines[i], ends[i]);
        let hero = hero(&ws, ends[i], counts);
        let hero_len = hero.map_or(0, |h| letters(&ws[h].1));
        let fits = |l: Layout| l.fits(ws.len(), hero_len);
        let before = prev.map(|p| p.layout);
        let last_place = prev.map(|p| (p.x, p.y));
        info.push((if chorus { runs } else { 0 }, ws.len(), energy.between(f64::from(lines[i].ms), ends[i])));
        // In a duet each singer keeps a side: the first the left, the second the right, together the middle.
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
                FALLBACK.into_iter().find(|l| Some(*l) != before && *l != next).unwrap_or(Layout::Stack)
            };
            k += 1;
            // The signature's row on the singer's side, another row when the line before was there.
            let (x, y) = match places[at] {
                (_, y) if duet && Some((side, y)) == last_place => (side, if y == 1 { -1 } else { 1 }),
                (_, y) if duet => (side, y),
                p => p,
            };
            Shot {
                layout,
                x,
                y,
                hero,
                enter: enters[at],
                exit: exits[at],
                level: run.min(MAX_LEVEL),
                paper: Paper::Dark,
                script: false,
                singer,
                answer,
                cont: false,
            }
        } else {
            // A verse line right before a chorus keeps the middle free for its anthem.
            let to_chorus =
                order.get(n + 1).is_some_and(|&(nsi, nj, ngap)| !ngap && nj == 0 && sections[nsi].kind == Kind::Chorus);
            let calm = s.kind != Kind::Editorial;
            let before_line = if after_gap { None } else { j.checked_sub(1).map(|p| s.lines[p]) };
            let cont = !duet
                && !prev.is_some_and(|p| p.cont)
                && before_line.is_some_and(|p| continues(&lines[p], &lines[i], ends[p]));
            // The line's own look first: short lines go big, long ones run in rows, a word sung again echoes,
            // a cry bursts, the rest of a sentence keeps its line's family; the seed only breaks ties.
            let n_words = ws.len();
            let text = lines[i].text.trim_end();
            let kin = if cont { prev.map(|p| family(p.layout)) } else { None };
            let favour = |l: Layout| -> u32 {
                let mut w = 1;
                if kin.is_some_and(|f| family(l) == f) {
                    w *= 6;
                }
                if n_words <= 3 && matches!(l, Layout::Hero | Layout::Stair | Layout::Vertical) {
                    w *= 4;
                }
                if n_words >= 8 && matches!(l, Layout::Rag | Layout::Climb | Layout::Cascade) {
                    w *= 4;
                }
                if text.ends_with(['!', '?']) && l == Layout::Burst {
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
                pool.push((Layout::Echo, 10 * favour(Layout::Echo)));
            }
            let layout = rng.weighted(&pool).or_else(|| pool.first().map(|p| p.0)).unwrap_or(Layout::Stack);
            // The places walk the scene's path one neighbour a line; a scene's start, a new sentence or a
            // downbeat may jump anywhere (the rest of a sentence never does).
            let allowed = |p: (i8, i8)| (!duet || p.0 == side) && Some(p) != last_place && !(to_chorus && p == CENTER);
            let near = |p: (i8, i8)| last_place.is_none_or(|l| (p.0 - l.0).abs() <= 1 && (p.1 - l.1).abs() <= 1);
            let free: Vec<(i8, i8)> = PLACES.iter().copied().filter(|p| allowed(*p)).collect();
            if before_line.is_none() {
                walk = (rng.below(PATHS.len()), 0, rng.coin());
            }
            let path = PATHS[walk.0];
            let jump = !cont
                && (before_line.is_none()
                    || before_line.is_some_and(|p| ends_sentence(&lines[p].text))
                    || on_bar(f64::from(lines[i].ms)));
            let place = if jump {
                let p = free[rng.below(free.len())];
                walk.1 =
                    (0..path.len()).min_by_key(|&q| (path[q].0 - p.0).abs().max((path[q].1 - p.1).abs())).unwrap_or(0);
                p
            } else {
                let len = path.len();
                let on_path = (1..=len)
                    .map(|d| if walk.2 { (walk.1 + d) % len } else { (walk.1 + len * 2 - d) % len })
                    .find(|&q| allowed(path[q]) && near(path[q]));
                match on_path {
                    Some(q) => {
                        walk.1 = q;
                        path[q]
                    }
                    // Off the path (a duet's side, the middle kept for the chorus): a neighbour, else anywhere.
                    None => {
                        let close: Vec<(i8, i8)> = free.iter().copied().filter(|p| near(*p)).collect();
                        let from = if close.is_empty() { &free } else { &close };
                        from[rng.below(from.len())]
                    }
                }
            };
            let e = energy.between(f64::from(lines[i].ms), ends[i]).unwrap_or(if calm { 0.4 } else { 0.65 });
            let pick_enter = |e: f64| -> [(Enter, u32); 6] {
                if e > 0.6 {
                    [
                        (Enter::Pop, 3),
                        (Enter::Stamp, 2),
                        (Enter::Slice, 2),
                        (Enter::Drift, 1),
                        (Enter::Letters, 1),
                        (Enter::Rise, 0),
                    ]
                } else if e < 0.35 {
                    [
                        (Enter::Letters, 3),
                        (Enter::Rise, 3),
                        (Enter::Drift, 1),
                        (Enter::Pop, 1),
                        (Enter::Slice, 0),
                        (Enter::Stamp, 0),
                    ]
                } else {
                    [
                        (Enter::Pop, 2),
                        (Enter::Rise, 2),
                        (Enter::Drift, 2),
                        (Enter::Letters, 1),
                        (Enter::Slice, 1),
                        (Enter::Stamp, 1),
                    ]
                }
            };
            let pick_exit = |e: f64| -> [(Exit, u32); 6] {
                if e > 0.6 {
                    [
                        (Exit::Scatter, 3),
                        (Exit::Zoom, 3),
                        (Exit::Steps, 2),
                        (Exit::Slide, 1),
                        (Exit::Fold, 1),
                        (Exit::Sink, 0),
                    ]
                } else if e < 0.35 {
                    [
                        (Exit::Sink, 3),
                        (Exit::Steps, 3),
                        (Exit::Slide, 2),
                        (Exit::Fold, 1),
                        (Exit::Zoom, 0),
                        (Exit::Scatter, 0),
                    ]
                } else {
                    [
                        (Exit::Sink, 2),
                        (Exit::Steps, 2),
                        (Exit::Slide, 2),
                        (Exit::Fold, 2),
                        (Exit::Zoom, 2),
                        (Exit::Scatter, 2),
                    ]
                }
            };
            let enters: Vec<(Enter, u32)> =
                pick_enter(e).into_iter().filter(|x| Some(x.0) != prev.map(|p| p.enter)).collect();
            // What covers the screen only with room before the next line; the block's last line leaves
            // stronger.
            let gap = order
                .get(n + 1)
                .map_or(f64::INFINITY, |&(nsi, nj, _)| f64::from(lines[sections[nsi].lines[nj]].ms) - ends[i]);
            let last_of_block = j + 1 == s.lines.len();
            let exits: Vec<(Exit, u32)> = pick_exit(e)
                .into_iter()
                .filter(|x| Some(x.0) != prev.map(|p| p.exit))
                .map(|(x, w)| match x {
                    Exit::Zoom | Exit::Scatter if gap < ROOM_MS => (x, 0),
                    Exit::Zoom | Exit::Scatter | Exit::Fold if last_of_block => (x, w * 2),
                    _ => (x, w),
                })
                .collect();
            Shot {
                layout,
                x: place.0,
                y: place.1,
                hero,
                enter: rng.weighted(&enters).unwrap_or(Enter::Pop),
                exit: rng.weighted(&exits).unwrap_or(Exit::Slide),
                level: 0,
                paper: Paper::Dark,
                script: false,
                singer,
                answer,
                cont,
            }
        };
        // The script face: always at the stair's ends; on a hero word now and then, never twice in a row.
        shot.script = match shot.layout {
            Layout::Stair => true,
            Layout::Hero => (3..=12).contains(&hero_len) && !prev.is_some_and(|p| p.script) && rng.coin(),
            _ => false,
        };
        // An answer types its letters; a group's words drift in from both sides.
        if answer {
            shot.enter = Enter::Letters;
        } else if duet && singer == 2 {
            shot.enter = Enter::Drift;
        }
        all[si].push(shot);
        prev = Some(shot);
    }
    // The peak: bars on the anthem of the strongest choruses after the first (of the only one when there is
    // one), one a chorus, at most MAX_BARS; without a chorus, the strongest line when it is loud enough.
    let late = info.iter().any(|x| x.0 >= 2);
    let mut cands: Vec<usize> = (0..order.len())
        .filter(|&n| {
            let (si, j, _) = order[n];
            all[si][j].layout == Layout::Anthem && Layout::Bar.fits(info[n].1, 0) && (!late || info[n].0 >= 2)
        })
        .collect();
    if runs == 0 {
        cands = (0..order.len())
            .filter(|&n| Layout::Bar.fits(info[n].1, 0) && info[n].2.is_some_and(|e| e > 0.8))
            .collect();
    }
    // The strongest first; without the analysis, the latest.
    cands.sort_by(|&a, &b| info[b].2.unwrap_or(0.0).total_cmp(&info[a].2.unwrap_or(0.0)).then(b.cmp(&a)));
    let mut bars: Vec<usize> = Vec::new();
    for n in cands {
        let limit = if runs == 0 { 1 } else { MAX_BARS };
        if bars.len() < limit && bars.iter().all(|&m| info[m].0 != info[n].0 || runs == 0) {
            bars.push(n);
        }
    }
    for &n in &bars {
        let (si, j, _) = order[n];
        all[si][j].layout = Layout::Bar;
        all[si][j].script = false;
    }
    // The paper cuts only on an event: a scene's start (every other driven verse on light paper), a bar
    // (light for its line), a duet's answer once the paper has stayed PAPER_HOLD_MS.
    let (mut paper, mut since, mut driven) = (Paper::Dark, 0.0, 0usize);
    for (n, &(si, j, _)) in order.iter().enumerate() {
        let s = &sections[si];
        let ms = f64::from(lines[s.lines[j]].ms);
        if j == 0 {
            paper = Paper::Dark;
            if s.kind == Kind::Editorial {
                driven += 1;
                if driven % 2 == 0 {
                    paper = Paper::Light;
                }
            }
            since = ms;
        }
        let shot = &mut all[si][j];
        if shot.answer && ms - since >= PAPER_HOLD_MS {
            paper = if paper == Paper::Dark { Paper::Light } else { Paper::Dark };
            since = ms;
        }
        shot.paper = if bars.contains(&n) { Paper::Light } else { paper };
    }
    let mut last = Camera::Push;
    for (s, shots) in sections.iter_mut().zip(all) {
        s.shots = shots;
        s.camera = match s.kind {
            Kind::Chorus | Kind::Opening => Camera::Push,
            Kind::Credits => Camera::Rise,
            _ => {
                let moves = [Camera::Pull, Camera::Left, Camera::Right, Camera::Rise, Camera::Roll, Camera::Push];
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
    let offset = facts.and_then(|f| f.beat_offset_ms).unwrap_or(0.0);
    // A cut at `t` moves to the nearest bar line within reach that lies between `lo` (the end of what is
    // still sung) and `hi` (the next line's lead), so it never makes a word late nor cuts one short.
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
        let head = snap((duration * 0.2).min(12_000.0), 0.0, duration);
        let tail = snap(duration - (duration * 0.2).min(15_000.0), head, duration);
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
        let lead = |i: usize| f64::from(lines[i].ms) - LEAD_MS;
        let first = snap(lead(sung[0]), 0.0, lead(sung[0]));
        if first >= MIN_OPENING_MS {
            out.push(section(Kind::Opening, 0.0, first, "before_first"));
        }
        // 3. Each block a scene, and an instrumental in a long gap after it.
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
        // 4. The credits, after the last line.
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
    // The words the song sings, for the emphasis words, and how fast they come, for the voice.
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
    let voice = voice(bpm, energy.1, per_s, &mut rng);
    let duet = [0, 1].iter().all(|&v| sung.iter().filter(|&&i| lines[i].voice == v).count() >= 2);
    shoot(&mut sections, lines, &ends, &energy, &counts, duet, beat.map(|b| (b, offset)), &mut rng);
    // How each scene is entered, from the one before it.
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
        voice,
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
        let fonts: Vec<Value> = FONTS
            .iter()
            .map(|(family, bytes)| {
                json!({ "family": family, "data": base64::engine::general_purpose::STANDARD.encode(bytes) })
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
    let timed = |ws: &crate::lyrics::Words| -> Value {
        if ws.is_empty() { Value::Null } else { ws.iter().map(|(ms, w)| json!([ms, w])).collect() }
    };
    // [start, text, words, singer, background vocals]
    let page_lines: Vec<Value> =
        lines.iter().map(|l| json!([l.ms, l.text, timed(&l.words), l.voice, timed(&l.bg)])).collect();
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
        // A verse lifts into the chorus; the gap after it is a frame, and the verse after it a return.
        assert_eq!(choruses[0].entry, Entry::Lift);
        assert_eq!(kinds(&s)[3], Kind::Instrumental);
        assert_eq!(
            s.sections.iter().map(|x| x.entry).skip(3).take(2).collect::<Vec<_>>(),
            [Entry::Frame, Entry::Return]
        );
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
    fn cuts_land_on_the_bar_without_cutting_a_line() {
        let lines = song();
        let s = direct(&lines, Some(&facts(60.0)), &track(60_000));
        // The first line at 10.1 s: the bar at 10 s is past its lead (9.95 s), so the cut stays there.
        assert_eq!(s.sections[0].kind, Kind::Opening);
        assert_eq!(s.sections[0].to_ms, 9950);
        // The chorus at 16.2 s moves back to the bar at 16 s, where the line before it ends.
        let chorus = s.sections.iter().find(|x| x.kind == Kind::Chorus).expect("a chorus");
        assert_eq!(chorus.from_ms, 16_000);
        // No scene of lines starts after its first line's lead, nor before the line before it ends.
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

    /// A longer song: two verses of four lines, a four-line chorus three times (the last after a gap), an
    /// unrepeated bridge, with word times on the chorus.
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

    /// The shots in order, each with whether a textless scene comes right before it.
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
    fn no_layout_or_place_twice_in_a_row() {
        let seeds = ["abcdefghijk", "dQw4w9WgXcQ", "zzzzzzzzzzz", "0", "kJQP7kiw5Fk", "x1y2z3", "seed", "another one"];
        for seed in seeds {
            for (lines, f) in [(song(), None), (long_song(), None), (long_song(), Some(facts(160.0)))] {
                let s = direct(&lines, f.as_ref(), &Track { seed, ..track(160_000) });
                let all = shots(&s);
                for w in all.windows(2) {
                    let ((a, _, _), (b, gap, _)) = (w[0], w[1]);
                    if gap {
                        continue;
                    }
                    assert_ne!(a.layout, b.layout, "{seed}: {all:?}");
                    assert_ne!((a.x, a.y), (b.x, b.y), "{seed}: {all:?}");
                }
                // The anthem is the chorus' own, and each layout only takes the lines it fits.
                for x in &s.sections {
                    for (&i, shot) in x.lines.iter().zip(&x.shots) {
                        assert!(shot.layout != Layout::Anthem || x.kind == Kind::Chorus);
                        let n = lines[i].text.split_whitespace().count();
                        assert!(shot.layout.fits(n, 0), "{:?} for {n} words", shot.layout);
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
            // The bars stand in for the anthem at the peak.
            let anthem = |l: Layout| if l == Layout::Bar { Layout::Anthem } else { l };
            let look = |r: &[Shot]| r.iter().map(|x| (anthem(x.layout), x.x, x.y, x.enter, x.exit)).collect::<Vec<_>>();
            assert_eq!(look(&runs[0]), look(&runs[1]));
            assert_eq!(look(&runs[0]), look(&runs[2]));
            assert_eq!(runs[0][0].layout, Layout::Anthem);
            assert_eq!((runs[0][0].x, runs[0][0].y), (0, 0));
            let levels: Vec<u8> = runs.iter().map(|r| r[0].level).collect();
            assert_eq!(levels, [1, 2, 3]);
            assert!(runs.iter().all(|r| r.iter().all(|x| x.level == r[0].level)));
            // Verses are never at level > 0.
            assert!(s.sections.iter().filter(|x| x.kind != Kind::Chorus).all(|x| x.shots.iter().all(|y| y.level == 0)));
        }
    }

    #[test]
    fn the_same_seed_writes_the_same_script() {
        let a = direct(&long_song(), Some(&facts(160.0)), &track(160_000));
        let b = direct(&long_song(), Some(&facts(160.0)), &track(160_000));
        assert_eq!(a, b);
        let looks = |seed: &'static str| {
            let s = direct(&long_song(), None, &Track { seed, ..track(160_000) });
            shots(&s).into_iter().map(|(x, _, _)| (x.layout, x.x, x.y)).collect::<Vec<_>>()
        };
        assert_eq!(looks("abcdefghijk"), looks("abcdefghijk"));
        assert_ne!(looks("abcdefghijk"), looks("dQw4w9WgXcQ"), "another seed, another show");
    }

    /// A duet: the first singer, the second answering, both together, never a line twice.
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
        for seed in ["abcdefghijk", "dQw4w9WgXcQ", "zzzzzzzzzzz", "seed"] {
            for f in [None, Some(facts(60.0))] {
                let s = direct(&lines, f.as_ref(), &Track { seed, ..track(60_000) });
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
                        assert!(!answer || shot.enter == Enter::Letters);
                        prev = Some(v);
                    }
                }
                for w in shots(&s).windows(2) {
                    assert!(w[1].1 || (w[0].0.x, w[0].0.y) != (w[1].0.x, w[1].0.y), "{seed}: {w:?}");
                }
            }
        }
        // One singer is not a duet.
        let solo: Vec<Line> = duet_song().into_iter().map(|l| Line { voice: 0, ..l }).collect();
        let s = direct(&solo, None, &track(60_000));
        assert!(!s.duet && shots(&s).iter().all(|(x, _, _)| x.singer == 0 && !x.answer));
    }

    #[test]
    fn the_bars_come_at_most_twice_on_the_strongest_choruses() {
        for seed in ["abcdefghijk", "dQw4w9WgXcQ", "zzzzzzzzzzz", "seed"] {
            for f in [None, Some(facts(160.0))] {
                let s = direct(&long_song(), f.as_ref(), &Track { seed, ..track(160_000) });
                let bars: Vec<Shot> =
                    shots(&s).into_iter().filter(|(x, _, _)| x.layout == Layout::Bar).map(|x| x.0).collect();
                assert!((1..=MAX_BARS).contains(&bars.len()), "{seed}: {bars:?}");
                // Never on the first chorus, one a chorus, on light paper.
                assert!(bars.iter().all(|x| x.level >= 2 && x.paper == Paper::Light), "{seed}: {bars:?}");
                assert!(bars.windows(2).all(|w| w[0].level != w[1].level), "{seed}: {bars:?}");
            }
        }
        // Without a chorus or the analysis, no peak to mark.
        let s = direct(&duet_song(), None, &track(60_000));
        assert!(shots(&s).iter().all(|(x, _, _)| x.layout != Layout::Bar));
    }

    #[test]
    fn the_paper_cuts_only_on_an_event() {
        let mut light = false;
        for seed in ["abcdefghijk", "dQw4w9WgXcQ", "zzzzzzzzzzz", "seed"] {
            for (lines, f, d) in
                [(long_song(), Some(facts(160.0)), 160_000), (long_song(), None, 160_000), (duet_song(), None, 60_000)]
            {
                let s = direct(&lines, f.as_ref(), &Track { seed, ..track(d) });
                for x in &s.sections {
                    for w in x.shots.windows(2) {
                        let event = w[0].layout == Layout::Bar || w[1].layout == Layout::Bar || w[1].answer;
                        assert!(w[0].paper == w[1].paper || event, "{seed}: {:?}", x.shots);
                    }
                    light |= x.shots.iter().any(|y| y.paper == Paper::Light && y.layout != Layout::Bar);
                }
            }
        }
        assert!(light, "some scene is on light paper");
    }

    #[test]
    fn the_emphasis_word_is_the_longest_held() {
        let ws: Vec<(f64, String)> = [(0.0, "I"), (200.0, "will"), (500.0, "wait"), (2500.0, "for"), (2700.0, "you")]
            .iter()
            .map(|(ms, w)| (*ms, (*w).to_owned()))
            .collect();
        assert_eq!(hero(&ws, 3000.0, &HashMap::new()), Some(2));
        // Short words only when there is nothing else.
        let short: Vec<(f64, String)> = vec![(0.0, "oh".into()), (100.0, "no".into())];
        assert_eq!(hero(&short, 2000.0, &HashMap::new()), Some(1));
        assert_eq!(hero(&[], 1000.0, &HashMap::new()), None);
    }

    #[test]
    fn the_voice_follows_tempo_and_loudness() {
        let with = |bpm: f64, level: u8| {
            let mut f = facts(160.0);
            f.bpm = Some(bpm);
            let bins = 1600;
            let bytes: Vec<u8> = (0..bins).flat_map(|b| [level.saturating_add((b % 7) as u8), level, 60]).collect();
            f.envelope = base64::engine::general_purpose::STANDARD.encode(bytes);
            direct(&long_song(), Some(&f), &track(160_000)).voice
        };
        assert_eq!(with(150.0, 235), Voice::Compressed);
        assert_eq!(with(68.0, 90), Voice::Expanded);
        // Without facts, a ballad's slow words are expanded and a fast flow is compressed.
        let slow: Vec<Line> = (0..10).map(|i| line(5000 + i * 6000, 5000 + i * 6000 + 5800, "so slow")).collect();
        assert_eq!(direct(&slow, None, &track(80_000)).voice, Voice::Expanded);
        let fast: Vec<Line> = (0..10)
            .map(|i| line(5000 + i * 2000, 5000 + i * 2000 + 1900, "words come so fast here in this one line now"))
            .collect();
        assert_eq!(direct(&fast, None, &track(40_000)).voice, Voice::Compressed);
    }
}
