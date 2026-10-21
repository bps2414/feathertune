//! `audio_facts`: the second command the site may call (capabilities/player-bridge.json). The
//! `audio` page module measures a track the first time it plays (track_analysis) and sends numbers
//! only: tempo, first beat, key, where the sound starts and ends, and a loudness envelope in three
//! bands. Never audio. Rust validates them, keeps them in `audio-facts/<id>.json` in the profile (at
//! most one write per track and session, at most `MAX_FILES` files, the oldest removed first) and
//! never acts on them or sends them anywhere: they come back to the page and to the Mix tab as data.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Webview};

/// Raw payload limit, before parsing.
const MAX_PAYLOAD: usize = 48 * 1024;
/// Three bands, 10 a second, for 15 minutes.
pub const MAX_ENVELOPE_BYTES: usize = 27_000;
const MAX_FILES: usize = 5000;
const MAX_SECONDS: f64 = 86_400.0;
const KEYS: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    pub id: String,
    pub duration_s: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat_offset_ms: Option<f64>,
    /// "A minor", "Eb major": estimated from the chroma, never certain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The key on the Camelot wheel ("8A").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camelot: Option<String>,
    pub lead_in_ms: f64,
    /// Where the sound ends; unknown when the end was not heard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_ms: Option<f64>,
    /// Where a track that fades out by itself was last loud (15 dB under its median); with `tail_ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_ms: Option<f64>,
    /// Base64 of u8 triples (low < 250 Hz, mid, high > 4 kHz), one per 100 ms, -60 dB = 0 to 0 dB = 255.
    pub envelope: String,
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The facts if every field is in range (also used for the files read back from the profile).
pub fn parse(json: &str) -> Option<Facts> {
    if json.len() > MAX_PAYLOAD {
        return None;
    }
    let f: Facts = serde_json::from_str(json).ok()?;
    let ms = f.duration_s * 1000.0;
    let in_track = |x: f64| x.is_finite() && (0.0..=ms).contains(&x);
    let ok = valid_id(&f.id)
        && f.duration_s.is_finite()
        && (1.0..=MAX_SECONDS).contains(&f.duration_s)
        && f.bpm.is_none_or(|b| b.is_finite() && (40.0..=250.0).contains(&b))
        && f.beat_offset_ms.is_none_or(|o| o.is_finite() && (0.0..=2000.0).contains(&o))
        && f.key.as_deref().is_none_or(valid_key)
        && f.camelot.as_deref().is_none_or(valid_camelot)
        && in_track(f.lead_in_ms)
        && f.tail_ms.is_none_or(|t| in_track(t) && t >= f.lead_in_ms)
        && f.fade_ms.is_none_or(|x| f.tail_ms.is_some_and(|t| in_track(x) && x <= t))
        && valid_envelope(&f.envelope);
    ok.then_some(f)
}

fn valid_key(key: &str) -> bool {
    key.split_once(' ').is_some_and(|(tonic, mode)| KEYS.contains(&tonic) && matches!(mode, "major" | "minor"))
}

fn valid_camelot(code: &str) -> bool {
    let (number, letter) = code.split_at(code.len().saturating_sub(1));
    matches!(letter, "A" | "B") && number.parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) && !number.starts_with('0')
}

fn valid_envelope(b64: &str) -> bool {
    let bytes = b64.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return false;
    }
    let pad = bytes.iter().rev().take_while(|&&b| b == b'=').count();
    let body = &bytes[..bytes.len() - pad];
    let decoded = bytes.len() / 4 * 3 - pad;
    pad <= 2
        && body.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/')
        && decoded <= MAX_ENVELOPE_BYTES
        && decoded.is_multiple_of(3)
}

/// The store in the profile, and the ids written in this session.
pub struct Store {
    dir: PathBuf,
    written: Mutex<HashSet<String>>,
}

impl Store {
    pub fn new(profile: &Path) -> Store {
        Store { dir: profile.join("audio-facts"), written: Mutex::default() }
    }

    pub fn load(&self, id: &str) -> Option<Facts> {
        if !valid_id(id) {
            return None;
        }
        parse(&std::fs::read_to_string(self.dir.join(format!("{id}.json"))).ok()?)
    }

    fn save(&self, facts: &Facts) {
        if !self.written.lock().unwrap().insert(facts.id.clone()) {
            return;
        }
        let _ = std::fs::create_dir_all(&self.dir);
        let json = serde_json::to_string(facts).expect("facts serialize");
        if let Err(e) = std::fs::write(self.dir.join(format!("{}.json", facts.id)), json) {
            eprintln!("audio facts: {e}");
        }
        self.prune();
    }

    /// Keeps the newest `MAX_FILES` files.
    fn prune(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut files: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| (e.metadata().and_then(|m| m.modified()).ok(), e.path()))
            .collect();
        if files.len() <= MAX_FILES {
            return;
        }
        files.sort();
        for (_, path) in &files[..files.len() - MAX_FILES] {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The command (see the module doc). Invalid payloads are dropped; nothing here triggers an action.
#[tauri::command]
pub fn audio_facts(webview: Webview, app: AppHandle, facts: String) {
    if webview.label() != "site" {
        return;
    }
    let Some(store) = app.try_state::<Store>() else { return };
    match parse(&facts) {
        Some(f) => store.save(&f),
        None => eprintln!("audio_facts: invalid payload dropped ({} bytes)", facts.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> serde_json::Value {
        serde_json::json!({
            "id": "dQw4w9WgXcQ", "duration_s": 213.1, "bpm": 113.2, "beat_offset_ms": 310.0,
            "key": "Ab major", "camelot": "4B", "lead_in_ms": 300.0, "tail_ms": 212_500.0, "envelope": "AAEC/w8A",
        })
    }

    fn with(key: &str, value: serde_json::Value) -> String {
        let mut v = sample();
        v[key] = value;
        v.to_string()
    }

    #[test]
    fn valid_facts_pass_and_round_trip() {
        let f = parse(&sample().to_string()).unwrap();
        assert_eq!(f.camelot.as_deref(), Some("4B"));
        assert_eq!(parse(&serde_json::to_string(&f).unwrap()), Some(f));
        let mut minimal = sample();
        for k in ["bpm", "beat_offset_ms", "key", "camelot", "tail_ms"] {
            minimal.as_object_mut().unwrap().remove(k);
        }
        assert!(parse(&minimal.to_string()).is_some());
    }

    #[test]
    fn out_of_range_facts_are_dropped() {
        for (key, value) in [
            ("id", serde_json::json!("../../x")),
            ("id", serde_json::json!("short")),
            ("duration_s", serde_json::json!(-1.0)),
            ("bpm", serde_json::json!(900.0)),
            ("beat_offset_ms", serde_json::json!(-5.0)),
            ("key", serde_json::json!("H major")),
            ("key", serde_json::json!("C dorian")),
            ("camelot", serde_json::json!("13A")),
            ("camelot", serde_json::json!("08A")),
            ("camelot", serde_json::json!("4C")),
            ("lead_in_ms", serde_json::json!(999_999_999.0)),
            ("tail_ms", serde_json::json!(100.0)),
            ("envelope", serde_json::json!("AAE")),
            ("envelope", serde_json::json!("AA<>")),
            ("envelope", serde_json::json!("AAA=")),
            ("extra", serde_json::json!(1)),
        ] {
            assert!(parse(&with(key, value.clone())).is_none(), "{key} = {value}");
        }
        let long = "A".repeat((MAX_ENVELOPE_BYTES + 3) / 3 * 4);
        assert!(parse(&with("envelope", serde_json::json!(long))).is_none());
        assert!(parse(&"x".repeat(MAX_PAYLOAD + 1)).is_none());
    }
}
