//! Central player state, fed by the page bridge (the `core` injection) through `player_state`.
//!
//! API (this is all that integrations such as Discord or Last.fm should use):
//!
//! - `app.state::<Player>()` gives the managed `Player`.
//! - `Player::current() -> Option<State>`: the last known state, `None` until something plays.
//! - `Player::subscribe() -> broadcast::Receiver<Event>`: every change from now on. Each `Event`
//!   carries the full `State`, so a consumer never needs to ask for it:
//!   - `Event::Track`: a new track started (or the first state after startup). It is keyed on
//!     title + artist; the album and the duration may still be empty here and arrive a moment
//!     later in an `Event::Progress`;
//!   - `Event::Playback`: same track, paused or resumed;
//!   - `Event::Progress`: same track and play state, new data: a seek, the late album/duration,
//!     or the page's position refresh (every 15 s while playing). Between events the position is
//!     not reported, so use `State::position_now()` for the live position.
//!
//!   The receiver is a `tokio::sync::broadcast` one: use `recv().await` in async code or
//!   `blocking_recv()` on a plain thread. A slow consumer gets `RecvError::Lagged` and should just
//!   keep receiving, since the next event has the full state again.
//!
//! Everything in `State` comes from the page and was validated here: strings are length-capped,
//! numbers are finite and in range, and `artwork` is an https URL on one of the image hosts below. It is
//! still display data only: never use it to build commands, paths or markup.

use std::sync::Mutex;
use std::time::Instant;

use serde::Deserialize;
use tauri::Url;
use tokio::sync::broadcast;

/// Raw payload limit, before parsing.
const MAX_PAYLOAD: usize = 8 * 1024;
const MAX_TEXT: usize = 512;
const MAX_URL: usize = 2048;
/// 24 h: longer than any track or mix.
const MAX_SECONDS: f64 = 86_400.0;
const ARTWORK_HOSTS: &[&str] = &["googleusercontent.com", "ggpht.com", "ytimg.com"];

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Video id (11 chars), when the page URL has one.
    pub id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: Option<String>,
    /// Seconds; 0 while unknown.
    pub duration: f64,
}

#[derive(Clone, Debug)]
pub struct State {
    pub track: Track,
    /// Seconds from the start of the track, as read at `at`.
    pub position: f64,
    pub paused: bool,
    pub at: Instant,
}

impl State {
    /// `position` moved forward by the time elapsed since it was read, if playing.
    pub fn position_now(&self) -> f64 {
        if self.paused {
            return self.position;
        }
        let p = self.position + self.at.elapsed().as_secs_f64();
        if self.track.duration > 0.0 { p.min(self.track.duration) } else { p }
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Track(State),
    Playback(State),
    Progress(State),
}

impl Event {
    pub fn state(&self) -> &State {
        match self {
            Event::Track(s) | Event::Playback(s) | Event::Progress(s) => s,
        }
    }
}

pub struct Player {
    current: Mutex<Option<State>>,
    tx: broadcast::Sender<Event>,
}

impl Player {
    pub fn new() -> Player {
        Player { current: Mutex::new(None), tx: broadcast::channel(32).0 }
    }

    #[allow(dead_code)] // for the integrations (Phase 3)
    pub fn current(&self) -> Option<State> {
        self.current.lock().unwrap().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    fn update(&self, new: State) {
        let mut current = self.current.lock().unwrap();
        let event = match current.as_ref() {
            Some(old) if old.track == new.track && old.paused == new.paused && old.position == new.position => return,
            // The album and duration arrive a moment after the title; that is the same track.
            Some(old) if old.track.title == new.track.title && old.track.artist == new.track.artist => {
                if old.paused != new.paused { Event::Playback(new.clone()) } else { Event::Progress(new.clone()) }
            }
            _ => Event::Track(new.clone()),
        };
        *current = Some(new);
        drop(current);
        let _ = self.tx.send(event);
    }
}

/// The one command the page may call (see `capabilities/`). It only stores data: invalid payloads
/// are dropped and nothing here triggers an action.
#[tauri::command]
pub fn player_state(state: String, player: tauri::State<'_, Player>) {
    match parse(&state) {
        Some(s) => player.update(s),
        None => eprintln!("player_state: invalid payload dropped ({} bytes)", state.len()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    id: Option<String>,
    title: String,
    artist: String,
    album: String,
    artwork: Option<String>,
    duration: f64,
    position: f64,
    paused: bool,
}

fn parse(json: &str) -> Option<State> {
    if json.len() > MAX_PAYLOAD {
        return None;
    }
    let p: Payload = serde_json::from_str(json).ok()?;
    let text =
        |s: String| (s.chars().count() <= MAX_TEXT && !s.chars().any(char::is_control)).then(|| s.trim().to_owned());
    let seconds = |x: f64| (x.is_finite() && (0.0..=MAX_SECONDS).contains(&x)).then_some(x);
    let id = match p.id {
        Some(id) if id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') => {
            Some(id)
        }
        Some(_) => return None,
        None => None,
    };
    let title = text(p.title).filter(|t| !t.is_empty())?;
    let artwork = match p.artwork {
        Some(a) => Some(artwork(&a)?),
        None => None,
    };
    Some(State {
        track: Track {
            id,
            title,
            artist: text(p.artist)?,
            album: text(p.album)?,
            artwork,
            duration: seconds(p.duration)?,
        },
        position: seconds(p.position)?,
        paused: p.paused,
        at: Instant::now(),
    })
}

fn artwork(url: &str) -> Option<String> {
    if url.len() > MAX_URL {
        return None;
    }
    let u = Url::parse(url).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    let ok = u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.port().is_none()
        && ARTWORK_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")));
    ok.then(|| u.into())
}
