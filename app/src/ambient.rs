//! Ambient mode (plugin `ambient_mode`), the Rust side: once per cover, a worker thread makes the
//! background the page module shows (injections/ambient/), a soft field of the cover's colors
//! (artwork::field), saturated and darkened, as a small PNG in a `data:` URL. The page shows it large
//! with no CSS blur, so its slow drift only moves a picture on the compositor instead of blurring the
//! whole window again at each step. Also carries game mode's `off` to the page.
//!
//! The page gets data only: `{ off, field }`, the field a `data:image/png;base64,` string or null (no
//! cover, or it failed to load). Nothing runs between covers.

use std::sync::Mutex;

use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;

use crate::artwork;
use crate::player::Player;

/// The cover is cut down to this many cells a side (the CSS blur it replaces was about that coarse)
/// and blown up to `SIDE` pixels; the page scales that up again, smoothly.
const CELLS: u32 = 8;
const SIDE: u32 = 64;
/// The CSS filter it replaces: `saturate(1.7) brightness(0.65)`.
const SATURATION: f32 = 1.7;
const BRIGHTNESS: f32 = 0.65;

/// What the page was last given: game mode's `off` and the field.
static PAGE: Mutex<(bool, Option<String>)> = Mutex::new((false, None));

pub fn start(app: &AppHandle) {
    let app = app.clone();
    let mut rx = app.state::<Player>().subscribe();
    std::thread::spawn(move || {
        // The cover URL last handled (`Some(None)`: a track without one).
        let mut last: Option<Option<String>> = None;
        loop {
            match rx.blocking_recv() {
                Ok(event) => {
                    let wanted = event.state().track.artwork.clone();
                    if last.as_ref() == Some(&wanted) {
                        continue;
                    }
                    last = Some(wanted.clone());
                    let field = wanted.and_then(|url| match background(&url) {
                        Ok(f) => Some(f),
                        Err(e) => {
                            eprintln!("ambient: no background: {e}");
                            None
                        }
                    });
                    PAGE.lock().unwrap().1 = field;
                    send(&app);
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// Game mode: the page module takes its layers out (`true`) or puts them back.
pub fn set_off(app: &AppHandle, off: bool) {
    PAGE.lock().unwrap().0 = off;
    send(app);
}

fn send(app: &AppHandle) {
    let (off, field) = PAGE.lock().unwrap().clone();
    crate::injections::send(app, "ambient", &json!({ "off": off, "field": field }));
}

fn background(url: &str) -> Result<String, String> {
    let art = artwork::load(url)?;
    let pixels = artwork::field(&art, CELLS, SIDE, SATURATION, BRIGHTNESS).map_err(|e| e.to_string())?;
    let png = artwork::png(&pixels, SIDE, SIDE).map_err(|e| e.to_string())?;
    Ok(format!("data:image/png;base64,{}", base64(&png)))
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]) } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }
}
