//! The current line of the synced lyrics, lit word by word when the entry has word times, and the next
//! one dimmer: shared by the floating lyrics (desktop_lyrics.rs) and the mini player's lyrics.
//!
//! - Fed by the `Player` state and the lines of `lyrics::Lyrics`. The time is `State::position_now()`
//!   with the page's 250 ms lead, so it changes lines with the site's Lyrics tab.
//! - `update` says when the line or the sung word changes next, for the caller's one-shot timer; a
//!   line change cross-fades for `FADE_MS`, during which the caller runs a frame timer (`fading`).
//! - Layouts and the lit brush are kept until their line, word or the render target changes.

use std::sync::Arc;
use std::time::Instant;

use tauri::{AppHandle, Manager};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush;
use windows::Win32::Graphics::DirectWrite::{DWRITE_TEXT_RANGE, IDWriteTextFormat, IDWriteTextLayout};
use windows_numerics::Vector2;

use crate::gfx::{self, point, rgba, with_alpha};
use crate::lyrics::{Line, Lyrics, Synced, track_key};
use crate::player::{Player, State};

/// The page lights a line this early (lyrics/page.js), so both change together.
const LEAD_MS: f64 = 250.0;
const FADE_MS: f32 = 320.0;
/// How far a line slides while it fades, in DIPs.
const SLIDE: f32 = 10.0;
/// The outline: the text drawn dark at these offsets under itself, then a soft drop below.
const OUTLINE: [(f32, f32); 8] =
    [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)];

/// A lyric line on screen: its layout, and for a line timed word by word a second one with the sung
/// words lit (made with the target's brush, so it is rebuilt with the target).
struct Text {
    text: String,
    plain: IDWriteTextLayout,
    /// Timed word by word: it starts dim and its words light up.
    words: bool,
    /// UTF-16 length of the sung words.
    sung: u32,
    lit: Option<(u32, IDWriteTextLayout)>,
}

/// What is shown: the track, the current line (-1: before the first) and its sung word.
struct Shown {
    key: String,
    index: isize,
    word: isize,
    current: Option<Text>,
    next: Option<IDWriteTextLayout>,
}

pub struct View {
    current_format: IDWriteTextFormat,
    next_format: IDWriteTextFormat,
    width: f32,
    current_height: f32,
    next_height: f32,
    /// The lit words' brush, with the target generation it was made on.
    lit: Option<(u32, ID2D1SolidColorBrush)>,
    shown: Option<Shown>,
    /// The content that is fading out, and since when.
    old: Option<(Shown, Instant)>,
}

/// What `View::update` found.
pub struct Update {
    pub playing: bool,
    /// Milliseconds until the line or the sung word changes (only while playing).
    pub next_ms: Option<f64>,
}

/// The current line (-1 before the first) and, in a line timed by word, the sung word (-1: none yet).
fn position(lines: &[Line], ms: f64) -> (isize, isize) {
    let index = lines.partition_point(|l| f64::from(l.ms) <= ms) as isize - 1;
    let word = usize::try_from(index)
        .ok()
        .map_or(-1, |i| lines[i].words.partition_point(|(at, _)| f64::from(*at) <= ms) as isize - 1);
    (index, word)
}

/// When the line or the word changes next, in ms of track time.
fn next_change(lines: &[Line], index: isize, ms: f64) -> Option<f64> {
    let next_line = lines.get((index + 1) as usize).map(|l| f64::from(l.ms));
    let next_word = usize::try_from(index)
        .ok()
        .and_then(|i| lines[i].words.iter().map(|(at, _)| f64::from(*at)).find(|at| *at > ms));
    [next_line, next_word].into_iter().flatten().reduce(f64::min)
}

/// UTF-16 length of the words up to `word`, within the line's text.
fn sung_len(line: &Line, word: isize) -> u32 {
    let n: usize = line.words.iter().take((word + 1).max(0) as usize).map(|(_, t)| t.encode_utf16().count()).sum();
    n.min(line.text.encode_utf16().count()) as u32
}

/// The player state and the lyrics of its track, when both are there.
fn wanted(app: &AppHandle) -> Option<(State, Arc<Synced>)> {
    let state = app.state::<Player>().current()?;
    let synced = app.state::<Lyrics>().current()?;
    (synced.key == track_key(&state.track)).then_some((state, synced))
}

impl View {
    /// The current line in `current_format` (wrapping) in `width` × `current_height` DIPs, the next one
    /// in `next_format` below it.
    pub fn new(
        current_format: IDWriteTextFormat,
        next_format: IDWriteTextFormat,
        width: f32,
        current_height: f32,
        next_height: f32,
    ) -> View {
        View { current_format, next_format, width, current_height, next_height, lit: None, shown: None, old: None }
    }

    fn text(&self, line: &Line, word: isize) -> Option<Text> {
        if line.text.is_empty() {
            return None;
        }
        let plain = gfx::text_layout(&line.text, &self.current_format, self.width, self.current_height).ok()?;
        let words = !line.words.is_empty();
        Some(Text {
            text: line.text.clone(),
            plain,
            words,
            sung: if words { sung_len(line, word) } else { 0 },
            lit: None,
        })
    }

    fn next_layout(&self, line: Option<&Line>) -> Option<IDWriteTextLayout> {
        let line = line.filter(|l| !l.text.is_empty())?;
        gfx::text_layout(&line.text, &self.next_format, self.width, self.next_height).ok()
    }

    /// Brings the content up to date with the player and the lyrics. While paused the content is
    /// cleared, unless `paused_too`.
    pub fn update(&mut self, app: &AppHandle, paused_too: bool) -> Update {
        let wanted = wanted(app);
        let playing = wanted.as_ref().is_some_and(|(s, _)| !s.paused);
        let mut next_ms = None;
        if let Some((state, synced)) = wanted.as_ref().filter(|_| playing || paused_too) {
            let lines = &synced.lines;
            let ms = state.position_now() * 1000.0 + LEAD_MS;
            let (index, word) = position(lines, ms);
            if playing {
                next_ms = next_change(lines, index, ms).map(|at| at - ms);
            }
            let same_line = self.shown.as_ref().is_some_and(|s| s.key == synced.key && s.index == index);
            if !same_line {
                let line = usize::try_from(index).ok().map(|i| &lines[i]);
                let shown = Shown {
                    key: synced.key.clone(),
                    index,
                    word,
                    current: line.and_then(|l| self.text(l, word)),
                    next: self.next_layout(lines.get((index + 1) as usize)),
                };
                // Only a line change within the track cross-fades; a new track just appears.
                let old = self.shown.replace(shown).filter(|s| s.key == synced.key);
                self.old = old.map(|s| (s, Instant::now()));
            } else if let Some(shown) = self.shown.as_mut().filter(|s| s.word != word) {
                shown.word = word;
                if let (Some(text), Ok(i)) = (shown.current.as_mut(), usize::try_from(index)) {
                    text.sung = sung_len(&lines[i], word);
                    text.lit = None;
                }
            }
        } else {
            self.shown = None;
            self.old = None;
        }
        Update { playing, next_ms }
    }

    /// Nothing to show: no lyrics for the track (or paused, without `paused_too`).
    pub fn is_empty(&self) -> bool {
        self.shown.is_none()
    }

    pub fn has_text(&self) -> bool {
        self.shown.as_ref().is_some_and(|s| s.current.is_some() || s.next.is_some())
    }

    /// Whether a cross-fade is still running (the caller's frame timer runs only then).
    pub fn fading(&mut self) -> bool {
        if self.old.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() * 1000.0 >= FADE_MS) {
            self.old = None;
        }
        self.old.is_some()
    }

    /// Draws the current line at (`x`, `top`) and the next one below it, in `color`; `outline` adds a
    /// dark outline so it reads on any background.
    pub fn draw(&mut self, c: &gfx::Canvas, x: f32, top: f32, color: D2D1_COLOR_F, outline: bool) {
        let t = self.old.as_ref().map_or(1.0, |(_, at)| (at.elapsed().as_secs_f32() * 1000.0 / FADE_MS).min(1.0));
        let t = 1.0 - (1.0 - t) * (1.0 - t);
        if self.lit.as_ref().is_none_or(|(g, _)| *g != c.generation) {
            self.lit = c.solid(color).map(|b| (c.generation, b));
        }
        let style = Style {
            format: &self.current_format,
            lit: self.lit.as_ref().map(|(_, b)| b),
            color,
            outline,
            x,
            top,
            next_top: top + self.current_height,
        };
        if let Some((old, _)) = self.old.as_mut() {
            paint(c, old, &style, 1.0 - t, -SLIDE * t);
        }
        if let Some(shown) = self.shown.as_mut() {
            paint(c, shown, &style, t, SLIDE * (1.0 - t));
        }
    }
}

struct Style<'a> {
    format: &'a IDWriteTextFormat,
    lit: Option<&'a ID2D1SolidColorBrush>,
    color: D2D1_COLOR_F,
    outline: bool,
    x: f32,
    top: f32,
    next_top: f32,
}

/// One content (current and next line) at `alpha`, moved down by `dy`.
fn paint(c: &gfx::Canvas, s: &mut Shown, style: &Style, alpha: f32, dy: f32) {
    if alpha <= 0.0 {
        return;
    }
    if let Some(text) = s.current.as_mut() {
        let origin = point(style.x, style.top + dy);
        if style.outline {
            outlined(c, &text.plain, origin, alpha);
        }
        if !text.words {
            c.layout(&text.plain, origin, with_alpha(style.color, alpha));
        } else {
            // Dim, then the sung words over it (the rest of the lit copy is drawn transparent).
            c.layout(&text.plain, origin, with_alpha(style.color, 0.5 * alpha));
            if let Some(brush) = style.lit.filter(|_| text.sung > 0) {
                if text.lit.as_ref().is_none_or(|(g, _)| *g != c.generation) {
                    text.lit = lit(text, style.format, brush).map(|l| (c.generation, l));
                }
                if let Some((_, layout)) = &text.lit {
                    unsafe {
                        brush.SetColor(&style.color);
                        brush.SetOpacity(alpha);
                    }
                    c.layout(layout, origin, rgba(0, 0.0));
                }
            }
        }
    }
    if let Some(next) = &s.next {
        let origin = point(style.x, style.next_top + dy);
        if style.outline {
            outlined(c, next, origin, alpha * 0.8);
        }
        c.layout(next, origin, with_alpha(style.color, 0.6 * alpha));
    }
}

/// The dark outline and drop under a layout.
fn outlined(c: &gfx::Canvas, layout: &IDWriteTextLayout, origin: Vector2, alpha: f32) {
    for (dx, dy) in OUTLINE {
        c.layout(layout, point(origin.X + dx, origin.Y + dy), rgba(0x000000, 0.5 * alpha));
    }
    c.layout(layout, point(origin.X, origin.Y + 2.5), rgba(0x000000, 0.35 * alpha));
}

/// A copy of the line's layout with the sung words in `brush`.
fn lit(text: &Text, format: &IDWriteTextFormat, brush: &ID2D1SolidColorBrush) -> Option<IDWriteTextLayout> {
    let (width, height) = unsafe { (text.plain.GetMaxWidth(), text.plain.GetMaxHeight()) };
    let layout = gfx::text_layout(&text.text, format, width, height).ok()?;
    unsafe { layout.SetDrawingEffect(brush, DWRITE_TEXT_RANGE { startPosition: 0, length: text.sung }).ok()? };
    Some(layout)
}
