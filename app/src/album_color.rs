//! Album color (plugin `album_color`): once per cover, a worker thread loads the artwork (artwork.rs,
//! the same download and WIC decoder as the mini player), picks a small palette from it and applies it:
//! - the title bar and the mini player get a `gfx::Theme` (deep ink and four glows in the cover's
//!   colors) that flows in over 0.7 s;
//! - the page gets the dominant color as data (`{accent, ink, onAccent}`) for the `theme` injection,
//!   which turns it into CSS variables (injections/theme/).
//!
//! Nothing runs on the UI thread, and nothing runs between tracks. Readability: the ink is always
//! dark (white text on it), and the glows' luminance is capped so white text over them keeps its
//! contrast. A track without a cover, or a failed download, goes back to the default look.

use std::sync::Mutex;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;

use crate::artwork::{self, Art};
use crate::gfx::{self, Theme, luminance};
use crate::player::Player;

/// Relative luminance above which a glow is darkened: white text over it keeps about 4.5:1.
const GLOW_MAX_LUMINANCE: f32 = 0.22;
/// Saturation under which a color is treated as gray (its hue means nothing) and left gray.
const GRAY: f32 = 0.12;

/// The current cover's page colors (accent, ink, text on the accent; `#rrggbb`), for the phone remote. Empty
/// without a cover.
static COLORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn colors() -> Vec<String> {
    COLORS.lock().unwrap().clone()
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    let mut rx = app.state::<Player>().subscribe();
    std::thread::spawn(move || {
        // The cover URL last handled (`Some(None)`: a track without one); a new track on the same
        // cover (an album) costs nothing.
        let mut last: Option<Option<String>> = None;
        loop {
            match rx.blocking_recv() {
                Ok(event) => {
                    let wanted = event.state().track.artwork.clone();
                    if last.as_ref() == Some(&wanted) {
                        continue;
                    }
                    last = Some(wanted.clone());
                    let look = wanted.and_then(|url| match artwork::load(&url) {
                        Ok(art) => Some(look(&art)),
                        Err(e) => {
                            eprintln!("album color: artwork not loaded: {e}");
                            None
                        }
                    });
                    apply(&app, look);
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn apply(app: &AppHandle, look: Option<(Theme, Value)>) {
    let (theme, page) = match look {
        Some((theme, page)) => (Some(theme), page),
        None => (None, Value::Null),
    };
    gfx::set_theme(theme);
    crate::titlebar::refresh(app);
    crate::miniplayer::refresh();
    crate::injections::send(app, "theme", &page);
    *COLORS.lock().unwrap() =
        ["accent", "ink", "onAccent"].iter().filter_map(|k| page.get(k)?.as_str().map(String::from)).collect();
    crate::remote::refresh(app);
}

/// The theme and the page data of a cover.
fn look(art: &Art) -> (Theme, Value) {
    let found = dominant_colors(art);
    let main = found[0];
    let (hue, saturation, _) = to_hsl(main);
    let gray = saturation < GRAY;
    let vivid = |s: f32| if gray { s } else { s.clamp(0.5, 0.9) };

    // Fewer than four distinct colors: the others are turned hues of the first.
    let glows: [u32; 4] = std::array::from_fn(|i| {
        let c = found.get(i).copied().unwrap_or_else(|| {
            let (h, s, l) = to_hsl(main);
            from_hsl(h + [40.0, -40.0, 80.0][i - 1], s, l)
        });
        glow(c)
    });
    let ink = from_hsl(hue, if gray { saturation } else { saturation.clamp(0.25, 0.6) }, 0.08);

    // The page's accent sits on the dark page: light enough to stand out from the ink.
    let mut lightness = 0.62;
    let mut accent = from_hsl(hue, vivid(saturation), lightness);
    while luminance(accent) < 0.18 && lightness < 0.85 {
        lightness += 0.03;
        accent = from_hsl(hue, vivid(saturation), lightness);
    }
    let l = luminance(accent);
    // Whichever of white and black reads better on the accent.
    let on_accent = if 1.05 / (l + 0.05) >= (l + 0.05) / 0.05 { 0xffffff } else { 0x000000 };
    let hex = |c: u32| format!("#{c:06x}");
    let page = json!({ "accent": hex(accent), "ink": hex(ink), "onAccent": hex(on_accent) });
    (Theme { ink, glows }, page)
}

/// A glow color: vivid, and dark enough for white text over it.
fn glow(c: u32) -> u32 {
    let (h, s, l) = to_hsl(c);
    let s = if s < GRAY { s } else { s.clamp(0.5, 0.9) };
    let mut l = l.clamp(0.35, 0.55);
    let mut out = from_hsl(h, s, l);
    while luminance(out) > GLOW_MAX_LUMINANCE && l > 0.2 {
        l -= 0.03;
        out = from_hsl(h, s, l);
    }
    out
}

/// Up to four distinct colors of the cover, the dominant first. The picture is binned to 16 levels per
/// channel; a bin scores by how many pixels it holds, weighted toward saturated, mid-light colors, so a
/// big dark or white background does not win over the cover's actual color.
fn dominant_colors(art: &Art) -> Vec<u32> {
    // The center square (video thumbnails are 16:9 with the cover in the middle).
    let side = art.width.min(art.height);
    let (x0, y0) = ((art.width - side) / 2, (art.height - side) / 2);
    // Per bin: pixel count and the sums of r, g, b.
    let mut bins = vec![[0u32; 4]; 4096];
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            let i = ((y * art.width + x) * 4) as usize;
            let (b, g, r) = (art.pixels[i], art.pixels[i + 1], art.pixels[i + 2]);
            let bin = &mut bins[(usize::from(r >> 4) << 8) | (usize::from(g >> 4) << 4) | usize::from(b >> 4)];
            *bin = [bin[0] + 1, bin[1] + u32::from(r), bin[2] + u32::from(g), bin[3] + u32::from(b)];
        }
    }
    let mut scored: Vec<(f32, u32)> = bins
        .iter()
        .filter(|b| b[0] > 0)
        .map(|b| {
            let avg = |sum: u32| sum / b[0];
            let color = avg(b[1]) << 16 | avg(b[2]) << 8 | avg(b[3]);
            let (_, s, l) = to_hsl(color);
            let mid = (1.0 - (2.0 * l - 1.0).powi(2)).max(0.02);
            (b[0] as f32 * (0.05 + s).powi(2) * mid, color)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut picked: Vec<u32> = Vec::new();
    for (_, color) in scored {
        if picked.iter().all(|p| distance(*p, color) > 70.0) {
            picked.push(color);
            if picked.len() == 4 {
                break;
            }
        }
    }
    if picked.is_empty() {
        picked.push(0x6b3bf0);
    }
    picked
}

/// Euclidean distance in RGB, 0..442.
fn distance(a: u32, b: u32) -> f32 {
    let d = |shift: u32| (((a >> shift) & 0xff) as f32 - ((b >> shift) & 0xff) as f32).powi(2);
    (d(16) + d(8) + d(0)).sqrt()
}

/// Hue in degrees (0..360), saturation and lightness (0..1).
fn to_hsl(c: u32) -> (f32, f32, f32) {
    let ch = |shift: u32| ((c >> shift) & 0xff) as f32 / 255.0;
    let (r, g, b) = (ch(16), ch(8), ch(0));
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(r) << 16 | byte(g) << 8 | byte(b)
}
