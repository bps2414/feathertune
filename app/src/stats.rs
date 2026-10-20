//! Listening stats (plugin `listening_stats`): every listen of at least `MIN_RECORD` of played time is
//! appended as one JSON line to `stats/plays.jsonl` in the profile (start time, title, artist, album,
//! length and seconds played). Nothing leaves the PC: the file is only read back here, for the Stats tab
//! of the settings window (`summary`), the CSV export and "clear history".
//!
//! - Played time counts only while playing, like the Last.fm scrobble point. A track played again from
//!   the start (repeat one) is a new listen.
//! - A listen is a play when at least half the track or 30 s was played, a skip otherwise.
//! - The listen in progress is written when the track changes and when the app quits from the tray or
//!   restarts; the summary counts it too.
//! - The summary is computed from the whole file on each request (about 20 000 lines a year, a few MB);
//!   days and hours are local, from the offset the settings page passes.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;

use crate::player::{Event, Player, State};

const MIN_RECORD: Duration = Duration::from_secs(10);
const PLAY_SECONDS: f64 = 30.0;
const TOP: usize = 10;
const DAY: i64 = 86_400;

#[derive(Serialize, Deserialize, Clone)]
struct Record {
    /// Unix seconds when the listen started.
    at: i64,
    title: String,
    artist: String,
    #[serde(default)]
    album: String,
    /// Track length in seconds, 0 when unknown.
    #[serde(default)]
    duration: u32,
    /// Seconds really played.
    played: u32,
}

impl Record {
    fn is_play(&self) -> bool {
        let needed = if self.duration > 0 { (f64::from(self.duration) / 2.0).min(PLAY_SECONDS) } else { PLAY_SECONDS };
        f64::from(self.played) >= needed
    }
}

/// The listen in progress.
struct Listen {
    record: Record,
    played: Duration,
    /// Set while playing: the time since then counts too.
    since: Option<Instant>,
    last: State,
}

impl Listen {
    fn new(state: &State) -> Listen {
        let t = &state.track;
        Listen {
            record: Record {
                at: unix_secs() - state.position_now() as i64,
                title: t.title.clone(),
                artist: t.artist.clone(),
                album: t.album.clone(),
                duration: t.duration.round() as u32,
                played: 0,
            },
            played: Duration::ZERO,
            since: (!state.paused).then(Instant::now),
            last: state.clone(),
        }
    }

    fn played(&self) -> Duration {
        self.played + self.since.map_or(Duration::ZERO, |s| s.elapsed())
    }

    /// The record of the listen so far, if it is long enough to keep.
    fn finished(&self) -> Option<Record> {
        let played = self.played();
        (played >= MIN_RECORD).then(|| Record { played: played.as_secs() as u32, ..self.record.clone() })
    }
}

/// Managed state: the file and the listen in progress.
pub struct Stats {
    path: PathBuf,
    current: Mutex<Option<Listen>>,
}

pub fn start(app: &AppHandle, profile: &std::path::Path) {
    app.manage(Stats { path: profile.join("stats").join("plays.jsonl"), current: Mutex::new(None) });
    let mut rx = app.state::<Player>().subscribe();
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            match rx.blocking_recv() {
                Ok(event) => app.state::<Stats>().handle(&event),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// Writes the listen in progress (the app is about to quit or restart). Any thread.
pub fn flush(app: &AppHandle) {
    if let Some(stats) = app.try_state::<Stats>() {
        let listen = stats.current.lock().unwrap().take();
        if let Some(record) = listen.and_then(|l| l.finished()) {
            stats.append(&record);
        }
    }
}

impl Stats {
    fn handle(&self, event: &Event) {
        let state = event.state();
        let mut current = self.current.lock().unwrap();
        let restart = match (event, current.as_ref()) {
            (Event::Track(_), _) | (_, None) => true,
            // Same track, back near the start after a real listen: it plays again (repeat one).
            (Event::Progress(_), Some(l)) => {
                !state.paused && state.position_now() + 10.0 < l.last.position_now() && l.played() >= MIN_RECORD
            }
            (Event::Playback(_), Some(_)) => false,
        };
        if restart {
            let old = current.replace(Listen::new(state));
            drop(current);
            if let Some(record) = old.and_then(|l| l.finished()) {
                self.append(&record);
            }
            return;
        }
        let Some(l) = current.as_mut() else { return };
        match (state.paused, l.since) {
            (true, Some(since)) => {
                l.played += since.elapsed();
                l.since = None;
            }
            (false, None) => l.since = Some(Instant::now()),
            _ => {}
        }
        if l.record.duration == 0 {
            l.record.duration = state.track.duration.round() as u32;
        }
        if l.record.album.is_empty() {
            l.record.album = state.track.album.clone();
        }
        l.last = state.clone();
    }

    fn append(&self, record: &Record) {
        let Ok(line) = serde_json::to_string(record) else { return };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
            writeln!(file, "{line}")
        };
        if let Err(e) = write() {
            eprintln!("stats: listen not written: {e}");
        }
    }

    /// Every record, the listen in progress last. Lines that do not parse are skipped.
    fn records(&self) -> Vec<Record> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut records: Vec<Record> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        if let Some(r) = self.current.lock().unwrap().as_ref().and_then(Listen::finished) {
            records.push(r);
        }
        records
    }

    /// Removes the history and forgets the listen in progress.
    pub fn clear(&self) -> Result<(), String> {
        *self.current.lock().unwrap() = None;
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("could not remove the history: {e}")),
            _ => Ok(()),
        }
    }

    /// Writes the history as CSV to `path`.
    pub fn export(&self, path: &std::path::Path) -> Result<(), String> {
        let csv = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::from("started_utc,title,artist,album,length_s,played_s\n");
        for r in self.records() {
            out.push_str(&format!(
                "{},{},{},{},{},{}\n",
                r.at,
                csv(&r.title),
                csv(&r.artist),
                csv(&r.album),
                r.duration,
                r.played
            ));
        }
        std::fs::write(path, out).map_err(|e| format!("could not write {}: {e}", path.display()))
    }

    /// The Stats tab: totals, a time series, an hour × weekday map and the top lists for `range`
    /// (`week`, `month`, `year` or `all`). `offset` is the page's `Date.getTimezoneOffset()` (minutes).
    pub fn summary(&self, range: &str, offset: i64) -> Value {
        let records = self.records();
        let local = |at: i64| at - offset * 60;
        let today = local(unix_secs()).div_euclid(DAY);
        let first_day = match range {
            "week" => today - 6,
            "month" => today - 29,
            "year" => today - 364,
            _ => records.iter().map(|r| local(r.at).div_euclid(DAY)).min().unwrap_or(today),
        };
        let in_range: Vec<&Record> = records.iter().filter(|r| local(r.at).div_euclid(DAY) >= first_day).collect();

        let mut seconds = 0u64;
        let mut plays = 0u32;
        let mut artists: HashMap<&str, (u64, u32)> = HashMap::new();
        let mut tracks: HashMap<(&str, &str), (u64, u32)> = HashMap::new();
        let mut albums: HashMap<(&str, &str), (u64, u32)> = HashMap::new();
        let mut heat = [[0u64; 24]; 7];
        for r in &in_range {
            let s = u64::from(r.played);
            let play = u32::from(r.is_play());
            seconds += s;
            plays += play;
            let add = |e: &mut (u64, u32)| {
                e.0 += s;
                e.1 += play;
            };
            if !r.artist.is_empty() {
                add(artists.entry(&r.artist).or_default());
            }
            add(tracks.entry((&r.title, &r.artist)).or_default());
            if !r.album.is_empty() {
                add(albums.entry((&r.album, &r.artist)).or_default());
            }
            let t = local(r.at);
            // Monday first: day 0 (1970-01-01) was a Thursday.
            let weekday = (t.div_euclid(DAY) + 3).rem_euclid(7) as usize;
            heat[weekday][(t.rem_euclid(DAY) / 3600) as usize] += s;
        }
        let (distinct_tracks, distinct_artists) = (tracks.len(), artists.len());
        let top_artists: Vec<Value> =
            top(artists).into_iter().map(|(name, (s, p))| json!({ "name": name, "seconds": s, "plays": p })).collect();
        let top_tracks: Vec<Value> = top(tracks)
            .into_iter()
            .map(|((title, artist), (s, p))| json!({ "name": title, "detail": artist, "seconds": s, "plays": p }))
            .collect();
        let top_albums: Vec<Value> = top(albums)
            .into_iter()
            .map(|((album, artist), (s, p))| json!({ "name": album, "detail": artist, "seconds": s, "plays": p }))
            .collect();

        // By day for a week or a month, by month for longer.
        let by_month = matches!(range, "year" | "all");
        let bucket = |day: i64| {
            if by_month {
                let (y, m) = civil(day);
                y * 12 + i64::from(m) - 1
            } else {
                day
            }
        };
        let (from, to) = (bucket(first_day), bucket(today));
        let mut series: Vec<(i64, u64)> = (from..=to).map(|k| (k, 0)).collect();
        for r in &in_range {
            if let Some(e) = series.get_mut((bucket(local(r.at).div_euclid(DAY)) - from) as usize) {
                e.1 += u64::from(r.played);
            }
        }
        let series: Vec<Value> = series
            .into_iter()
            .map(|(k, s)| {
                if by_month {
                    json!({ "year": k.div_euclid(12), "month": k.rem_euclid(12) + 1, "seconds": s })
                } else {
                    json!({ "day": k, "seconds": s })
                }
            })
            .collect();

        // Streaks count days with any listening, over the whole history.
        let mut days: Vec<i64> = records.iter().map(|r| local(r.at).div_euclid(DAY)).collect();
        days.sort_unstable();
        days.dedup();
        let (mut longest, mut run) = (0u32, 0u32);
        for (i, d) in days.iter().enumerate() {
            run = if i > 0 && days[i - 1] + 1 == *d { run + 1 } else { 1 };
            longest = longest.max(run);
        }
        let current = if days.last().is_some_and(|d| *d >= today - 1) { run } else { 0 };

        // Artists heard for the first time in the range.
        let mut first_heard: HashMap<&str, i64> = HashMap::new();
        for r in records.iter().filter(|r| !r.artist.is_empty()) {
            let e = first_heard.entry(&r.artist).or_insert(r.at);
            *e = (*e).min(r.at);
        }
        let mut new_artists: Vec<(&str, i64)> = if range == "all" {
            Vec::new()
        } else {
            first_heard.into_iter().filter(|(_, at)| local(*at).div_euclid(DAY) >= first_day).collect()
        };
        new_artists.sort_by_key(|(_, at)| std::cmp::Reverse(*at));

        json!({
            "seconds": seconds,
            "plays": plays,
            "skips": in_range.len() as u32 - plays,
            "tracks": distinct_tracks,
            "artists": distinct_artists,
            "new_artists": new_artists.len(),
            "new_artist_names": new_artists.iter().take(5).map(|(n, _)| *n).collect::<Vec<_>>(),
            "streak": current,
            "longest_streak": longest,
            "since": records.iter().map(|r| r.at).min(),
            "series": series,
            "heat": heat,
            "top_artists": top_artists,
            "top_tracks": top_tracks,
            "top_albums": top_albums,
        })
    }
}

/// The `TOP` entries with the most seconds.
fn top<K>(map: HashMap<K, (u64, u32)>) -> Vec<(K, (u64, u32))> {
    let mut v: Vec<_> = map.into_iter().collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1.0));
    v.truncate(TOP);
    v
}

fn unix_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Year and month (1..=12) of a day counted from 1970-01-01 (H. Hinnant's civil_from_days).
fn civil(days: i64) -> (i64, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32)
}
