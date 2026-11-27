//! Ambient mode (plugin `ambient_mode`), the Rust side: once per cover, a worker thread makes the
//! background the page module shows (injections/ambient/), a soft field of the cover's colors
//! (artwork::field), saturated and darkened, as a small PNG in a `data:` URL. The page shows it large
//! with no CSS blur, so its slow drift only moves a picture on the compositor instead of blurring the
//! whole window again at each step. With it goes the halo the page shows right behind the cover
//! (artwork::halo): the cover already blurred, saturated and faded out toward its sides, so the page runs
//! no blur or mask for it either (as a CSS filter the compositor drew it again at every frame that moved
//! anything over it, the lyrics' glide among them). Also carries game mode's `off` to the page.
//!
//! The page gets data only: `{ off, field, halo }`, each picture a `data:image/png;base64,` string or
//! null (no cover, or it failed to load). Nothing runs between covers.

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
/// The halo, and the CSS it replaces: `blur(40px) saturate(1.8) brightness(0.9)` under a radial mask
/// that is solid up to 45% of the way out. The blur was 40 px of a box 750 to 1200 px wide (1.7 times
/// the cover on screen), so about 4.4% of its side.
const HALO_SIDE: u32 = 96;
const HALO_SIGMA: f32 = 0.044 * HALO_SIDE as f32;
const HALO_SATURATION: f32 = 1.8;
const HALO_BRIGHTNESS: f32 = 0.9;
const HALO_SOLID: f32 = 0.45;

/// What the page was last given: game mode's `off`, the field and the halo.
static PAGE: Mutex<(bool, Option<String>, Option<String>)> = Mutex::new((false, None, None));

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
                    let (field, halo) = match wanted.map(|url| pictures(&url)) {
                        Some(Ok((field, halo))) => (Some(field), Some(halo)),
                        Some(Err(e)) => {
                            eprintln!("ambient: no background: {e}");
                            (None, None)
                        }
                        None => (None, None),
                    };
                    {
                        let mut page = PAGE.lock().unwrap();
                        page.1 = field;
                        page.2 = halo;
                    }
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
    let (off, field, halo) = PAGE.lock().unwrap().clone();
    crate::injections::send(app, "ambient", &json!({ "off": off, "field": field, "halo": halo }));
}

/// The field and the halo of the cover at `url`, each as a `data:` URL.
fn pictures(url: &str) -> Result<(String, String), String> {
    let art = artwork::load(url)?;
    let data = |pixels: windows::core::Result<Vec<u8>>, side: u32| {
        let png = artwork::png(&pixels.map_err(|e| e.to_string())?, side, side).map_err(|e| e.to_string())?;
        Ok::<_, String>(format!("data:image/png;base64,{}", base64(&png)))
    };
    let field = data(artwork::field(&art, CELLS, SIDE, SATURATION, BRIGHTNESS), SIDE)?;
    let halo = artwork::halo(&art, HALO_SIDE, HALO_SIGMA, HALO_SATURATION, HALO_BRIGHTNESS, HALO_SOLID);
    Ok((field, data(halo, HALO_SIDE)?))
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
