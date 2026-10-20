//! Synced lyrics (plugin `synced_lyrics`): once per track Rust asks LRCLIB (https://lrclib.net) for
//! the timed lyrics and hands the lines to the `lyrics` injection as data, which shows them in the
//! site's Lyrics tab following the <video> time. When LRCLIB has nothing the page is told nothing, so
//! the site's own lyrics stay as they are.
//!
//! - One `GET /api/get` per track (title, artist, album when known, duration), with a User-Agent that
//!   names the app, as LRCLIB asks. It waits for the duration, which arrives a moment after the title.
//! - Answers are cached in `lyrics-cache/` in the profile, one small JSON file per track, so a track
//!   already seen costs no request. "Not found" is cached too, for a week. Failures (network, server
//!   busy) are not cached and not retried before the track changes.
//! - The page never fetches anything and never calls Rust for this: it only receives
//!   `{ id, lines: [[ms, text, words?], ...] }` through `injections::send`, or `null` on a track change.
//!   `words` (`[[ms, text], ...]`) is there only when the lyrics are in Enhanced LRC, with a time
//!   before each word (`<mm:ss.xx>`); most LRCLIB entries are timed by line only.
//! - Plain (unsynced) lyrics are ignored: the site already shows those.
//! - The lines of the current track are also kept in Rust (`Lyrics`, managed state) for the floating
//!   lyrics (desktop_lyrics.rs). The lookup runs when either plugin is on; the page gets the lines only
//!   with `synced_lyrics` on.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::player::{Player, Track};

const API: &str = "https://lrclib.net/api/get";
const USER_AGENT: &str =
    concat!("feathertune/", env!("CARGO_PKG_VERSION"), " (https://github.com/bps2414/feathertune)");
const MAX_BODY: u64 = 512 * 1024;
const MAX_LINES: usize = 2000;
const MAX_LINE_CHARS: usize = 300;
const MAX_WORDS: usize = 100;
/// A cached "no synced lyrics" is asked again after this long.
const NOT_FOUND_TTL: u64 = 7 * 24 * 3600;
/// The cache holds at most this many files; over it, the oldest are removed down to `KEEP_FILES`.
const MAX_FILES: usize = 400;
const KEEP_FILES: usize = 300;

pub type Words = Vec<(u32, String)>;
type Lines = Vec<Line>;

#[derive(Clone, Serialize, Deserialize)]
pub struct Line {
    pub ms: u32,
    pub text: String,
    /// Each word with its own time and the spaces after it; empty when the line is timed as a whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Words,
}

/// The synced lines of the track `key` (`track_key`).
pub struct Synced {
    pub key: String,
    pub lines: Vec<Line>,
}

/// The synced lyrics of the current track, `None` while there are none (managed state).
#[derive(Default)]
pub struct Lyrics(Mutex<Option<Arc<Synced>>>);

impl Lyrics {
    pub fn current(&self) -> Option<Arc<Synced>> {
        self.0.lock().unwrap().clone()
    }

    fn set(&self, synced: Option<Synced>) {
        *self.0.lock().unwrap() = synced.map(Arc::new);
        crate::desktop_lyrics::refresh();
        crate::share_lyrics::refresh();
    }
}

impl Line {
    fn to_json(&self) -> Value {
        if self.words.is_empty() { json!([self.ms, self.text]) } else { json!([self.ms, self.text, self.words]) }
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    /// Unix seconds.
    at: u64,
    /// `None`: LRCLIB has no synced lyrics for the track.
    lines: Option<Lines>,
}

/// `page`: the `synced_lyrics` plugin is on, so the page gets the lines too.
pub fn start(app: &AppHandle, profile: &Path, page: bool) {
    let app = app.clone();
    let dir = profile.join("lyrics-cache");
    let mut rx = app.state::<Player>().subscribe();
    std::thread::spawn(move || {
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .tls_config(
                    TlsConfig::builder()
                        .provider(TlsProvider::NativeTls)
                        .root_certs(RootCerts::PlatformVerifier)
                        .build(),
                )
                .user_agent(USER_AGENT)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(10)))
                .build(),
        );
        // The track being shown, and whether it was already asked for.
        let mut current = String::new();
        let mut asked = false;
        loop {
            let state = match rx.blocking_recv() {
                Ok(event) => event.state().clone(),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            let key = track_key(&state.track);
            if key != current {
                current = key;
                asked = false;
                app.state::<Lyrics>().set(None);
                if page {
                    crate::injections::send(&app, "lyrics", &Value::Null);
                }
            }
            // The duration arrives after the title, in a later event.
            if asked || state.track.duration <= 0.0 {
                continue;
            }
            asked = true;
            let lines = lookup(&agent, &dir, &state.track);
            // The track may have changed during the request.
            let still = app.state::<Player>().current().is_some_and(|s| track_key(&s.track) == current);
            if let (Some(lines), true) = (lines, still) {
                if page {
                    let json: Vec<Value> = lines.iter().map(Line::to_json).collect();
                    let payload = json!({ "id": state.track.id, "lines": json });
                    crate::injections::send(&app, "lyrics", &payload);
                }
                app.state::<Lyrics>().set(Some(Synced { key: current.clone(), lines }));
            }
        }
    });
}

pub fn track_key(t: &Track) -> String {
    t.id.clone().unwrap_or_else(|| format!("{}\n{}", t.title, t.artist))
}

/// The synced lines of a track: from the cache, else from LRCLIB (and cached). `None`: none to show.
fn lookup(agent: &ureq::Agent, dir: &Path, track: &Track) -> Option<Lines> {
    let duration = track.duration.round() as u32;
    let file = cache_file(dir, track, duration);
    if let Some(entry) = read_entry(&file)
        && (entry.lines.is_some() || now().saturating_sub(entry.at) < NOT_FOUND_TTL)
    {
        return entry.lines;
    }
    let mut request = agent
        .get(API)
        .query("track_name", &track.title)
        .query("artist_name", &track.artist)
        .query("duration", duration.to_string());
    if !track.album.is_empty() {
        request = request.query("album_name", &track.album);
    }
    let lines = match request.call() {
        Ok(mut response) => {
            let body = response.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| e.to_string());
            let body = body.and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string()));
            match body {
                Ok(body) => body.get("syncedLyrics").and_then(Value::as_str).map(parse_lrc).filter(|l| !l.is_empty()),
                Err(e) => {
                    eprintln!("lyrics: unreadable answer: {e}");
                    return None;
                }
            }
        }
        Err(ureq::Error::StatusCode(404)) => None,
        Err(e) => {
            eprintln!("lyrics: request failed: {e}");
            return None;
        }
    };
    write_entry(&file, &Entry { at: now(), lines: lines.clone() });
    lines
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn cache_file(dir: &Path, track: &Track, duration: u32) -> PathBuf {
    let text = format!("{}\0{}\0{}\0{duration}", track.title, track.artist, track.album).to_lowercase();
    dir.join(format!("{:x}.json", md5::compute(text)))
}

fn read_entry(file: &Path) -> Option<Entry> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

fn write_entry(file: &Path, entry: &Entry) {
    let Some(dir) = file.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    match serde_json::to_vec(entry) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(file, bytes) {
                eprintln!("lyrics: cache not written: {e}");
            }
        }
        Err(e) => eprintln!("lyrics: cache not written: {e}"),
    }
    prune(dir);
}

/// Keeps the cache small: over `MAX_FILES`, the oldest go.
fn prune(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<_> =
        read.filter_map(Result::ok).filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= MAX_FILES {
        return;
    }
    files.sort();
    for (_, path) in &files[..files.len() - KEEP_FILES] {
        let _ = std::fs::remove_file(path);
    }
}

/// `[mm:ss.xx] text` lines (several stamps per line allowed) into lines by time, in milliseconds.
/// Metadata tags (`[ar:...]`) and lines without a time are dropped; the text is capped.
fn parse_lrc(text: &str) -> Lines {
    let mut lines: Lines = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_start();
        let mut times = Vec::new();
        while let Some(tagged) = rest.strip_prefix('[') {
            let Some((tag, after)) = tagged.split_once(']') else { break };
            let Some(ms) = parse_time(tag) else { break };
            times.push(ms);
            rest = after;
        }
        let (text, words) = parse_words(rest);
        for ms in times {
            lines.push(Line { ms, text: text.clone(), words: words.clone() });
        }
    }
    lines.sort_by_key(|l| l.ms);
    lines.truncate(MAX_LINES);
    lines
}

/// The text of a line without its word stamps, and its words when it has them (Enhanced LRC:
/// `<mm:ss.xx>word <mm:ss.xx>word`). Text before the first stamp is sung with the line.
fn parse_words(rest: &str) -> (String, Words) {
    let clean = |s: &str| -> String { s.chars().filter(|c| !c.is_control()).collect() };
    let mut plain = String::new();
    let mut words: Words = Vec::new();
    let mut stamped = false;
    let mut at = 0;
    let mut s = rest;
    loop {
        let next = stamp(s);
        let segment = clean(next.map_or(s, |(start, _, _)| &s[..start]));
        plain.push_str(&segment);
        if !segment.trim().is_empty() && words.len() < MAX_WORDS {
            words.push((at, segment));
        } else if let Some(last) = words.last_mut() {
            last.1.push_str(&segment);
        }
        let Some((_, end, ms)) = next else { break };
        stamped = true;
        at = ms;
        s = &s[end..];
    }
    let text: String = plain.trim().chars().take(MAX_LINE_CHARS).collect();
    if !stamped || text.is_empty() {
        return (text, Vec::new());
    }
    if let Some(first) = words.first_mut() {
        first.1 = first.1.trim_start().to_owned();
    }
    if let Some(last) = words.last_mut() {
        last.1 = last.1.trim_end().to_owned();
    }
    for word in &mut words {
        word.1 = word.1.chars().take(MAX_LINE_CHARS).collect();
    }
    (text, words)
}

/// The first `<mm:ss.xx>` in `s`: where it starts, where it ends and its time.
fn stamp(s: &str) -> Option<(usize, usize, u32)> {
    s.match_indices('<').find_map(|(start, _)| {
        let len = s[start + 1..].find('>')?;
        let ms = parse_time(&s[start + 1..start + 1 + len])?;
        Some((start, start + len + 2, ms))
    })
}

/// `mm:ss` or `mm:ss.xx`, in milliseconds.
fn parse_time(tag: &str) -> Option<u32> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes: u32 = minutes.parse().ok().filter(|m| *m < 600)?;
    let seconds: f64 = seconds.parse().ok().filter(|s| (0.0..60.0).contains(s))?;
    Some(minutes * 60_000 + (seconds * 1000.0).round() as u32)
}
