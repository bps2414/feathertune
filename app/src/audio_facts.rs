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
/// The algorithm's version the page writes now: 2 times the blocks by the audio clock and refines
/// the tempo over the whole track. Older files (no `v`) are still read, and measured again.
pub const VERSION: u8 = 2;
const MAX_PROBE: usize = 1024;

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    /// The algorithm's version (0: before versions).
    #[serde(default)]
    pub v: u8,
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
    let ms = (f.duration_s * 1000.0).round();
    let in_track = |x: f64| x.is_finite() && (0.0..=ms).contains(&x);
    let ok = f.v <= VERSION
        && valid_id(&f.id)
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
        {
            let mut written = self.written.lock().unwrap();
            if written.len() >= 1000 || !written.insert(facts.id.clone()) {
                return;
            }
        }
        let _ = std::fs::create_dir_all(&self.dir);
        let json = serde_json::to_string(facts).expect("facts serialize");
        if let Err(e) = crate::settings::write_atomic(&self.dir.join(format!("{}.json", facts.id)), json) {
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

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Tap {
    Worklet,
    Script,
}

/// How a measure went, only logged for the diagnostics: the tap, the blocks dropped by reason, the
/// blocks lost (gaps in the audio clock), the anchor's resyncs, its largest drift and where a jump
/// ended the beat window (s).
#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct Probe {
    tap: Tap,
    dropped_paused: u32,
    dropped_waiting: u32,
    dropped_ad: u32,
    gaps: u32,
    resyncs: u32,
    max_drift_ms: f64,
    #[serde(default)]
    beat_lost_s: Option<f64>,
}

impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let tap = match self.tap {
            Tap::Worklet => "worklet",
            Tap::Script => "script",
        };
        write!(
            f,
            "tap {tap}, dropped paused/waiting/ad {}/{}/{}, gaps {}, resyncs {}, max drift {:.0} ms, beat window lost at {:?} s",
            self.dropped_paused,
            self.dropped_waiting,
            self.dropped_ad,
            self.gaps,
            self.resyncs,
            self.max_drift_ms,
            self.beat_lost_s
        )
    }
}

fn parse_probe(json: &str) -> Option<Probe> {
    if json.len() > MAX_PROBE {
        return None;
    }
    let p: Probe = serde_json::from_str(json).ok()?;
    let ok = p.max_drift_ms.is_finite()
        && (0.0..=MAX_SECONDS * 1000.0).contains(&p.max_drift_ms)
        && p.beat_lost_s.is_none_or(|s| s.is_finite() && (0.0..=MAX_SECONDS).contains(&s));
    ok.then_some(p)
}

/// The command (see the module doc). Invalid payloads are dropped; nothing here triggers an action.
/// `probe`: how the measure went (`Probe`), only logged.
#[tauri::command]
pub fn audio_facts(webview: Webview, app: AppHandle, facts: String, probe: Option<String>) {
    if webview.label() != "site" {
        return;
    }
    let Some(store) = app.try_state::<Store>() else { return };
    match parse(&facts) {
        Some(f) => {
            let (bpm, id, v) = (f.bpm, &f.id, f.v);
            let probe = probe.as_deref().and_then(parse_probe).map_or_else(|| "no probe".to_owned(), |p| p.to_string());
            crate::diag::log("analysis", format_args!("{id}: v{v}, bpm {bpm:?}, {probe}"));
            store.save(&f);
            crate::stage::refresh();
        }
        None => eprintln!("audio_facts: invalid payload dropped ({} bytes)", facts.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> serde_json::Value {
        serde_json::json!({
            "v": 2, "id": "dQw4w9WgXcQ", "duration_s": 213.1, "bpm": 113.2, "beat_offset_ms": 310.0,
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
        let edge = serde_json::json!({
            "v": 2, "id": "dQw4w9WgXcQ", "duration_s": 1.001, "lead_in_ms": 0.0, "tail_ms": 1001.0, "envelope": "AAEC",
        });
        assert!(parse(&edge.to_string()).is_some());
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

    #[test]
    fn version_is_checked_and_old_files_still_read() {
        assert_eq!(parse(&sample().to_string()).unwrap().v, VERSION);
        let mut old = sample();
        old.as_object_mut().unwrap().remove("v");
        let f = parse(&old.to_string()).unwrap();
        assert_eq!((f.v, f.bpm), (0, Some(113.2)));
        for v in [serde_json::json!(VERSION + 1), serde_json::json!(-1), serde_json::json!(1.5), serde_json::json!("2")]
        {
            assert!(parse(&with("v", v.clone())).is_none(), "v = {v}");
        }
    }

    #[test]
    fn probe_is_checked() {
        let ok = serde_json::json!({
            "tap": "worklet", "dropped_paused": 3, "dropped_waiting": 0, "dropped_ad": 0, "gaps": 2,
            "resyncs": 1, "max_drift_ms": 41.0, "beat_lost_s": null,
        });
        let p = parse_probe(&ok.to_string()).unwrap();
        assert_eq!((p.tap, p.gaps, p.beat_lost_s), (Tap::Worklet, 2, None));
        for (key, value) in [
            ("tap", serde_json::json!("other")),
            ("gaps", serde_json::json!(-1)),
            ("resyncs", serde_json::json!(1.5)),
            ("max_drift_ms", serde_json::json!(-3.0)),
            ("beat_lost_s", serde_json::json!(1e9)),
            ("extra", serde_json::json!(1)),
        ] {
            let mut bad = ok.clone();
            bad[key] = value.clone();
            assert!(parse_probe(&bad.to_string()).is_none(), "{key} = {value}");
        }
        assert!(parse_probe(&"x".repeat(MAX_PROBE + 1)).is_none());
    }
}
