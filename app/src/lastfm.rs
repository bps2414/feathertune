//! Last.fm: now playing and scrobbles, fed only by the `Player` state (plugin `lastfm`).
//!
//! - API keys: `[lastfm] api_key` and `api_secret` in `config.toml` when both are set; otherwise the
//!   ones built into the binary from the CI secrets `FEATHERTUNE_LASTFM_API_KEY` and
//!   `FEATHERTUNE_LASTFM_API_SECRET` (only the `dist` job sets them); otherwise off. They are never
//!   in code or Git. Built-in keys can be read out of the exe, as in any desktop client.
//! - Sign-in is the desktop flow: `auth.getToken`, the browser opens the authorization page, then
//!   `auth.getSession` is polled until the user approves. The session key goes to the Windows
//!   Credential Manager (keyring) and nowhere else; the app never sees a password.
//! - `track.updateNowPlaying` when a track starts or resumes.
//! - `track.scrobble` once the track is longer than 30 s and half of it or 4 min has been *played*
//!   (pauses do not count), whichever comes first. The page only reports on changes, so a timer
//!   wakes the thread at that moment; there is no polling.
//! - A scrobble that cannot be sent (offline, Last.fm down) waits in `lastfm-queue.json` in the
//!   profile and is retried every 5 min, at startup and at the next scrobble. Last.fm ignores
//!   scrobbles older than 14 days, so those are dropped.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Url};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::player::{Event, Player, State};
use crate::settings::LastFm as Settings;

const API: &str = "https://ws.audioscrobbler.com/2.0/";
const AUTH_URL: &str = "https://www.last.fm/api/auth/";
const KEYRING_SERVICE: &str = "feathertune";
const KEYRING_USER: &str = "lastfm-session";
/// Shorter tracks are never scrobbled.
const MIN_LENGTH: f64 = 30.0;
const MAX_WAIT: f64 = 240.0;
const RETRY: Duration = Duration::from_secs(300);
const MAX_AGE_SECS: u64 = 14 * 24 * 3600;
/// Kept below 10 so the `artist[n]` parameter names sort the same as numbers.
const BATCH: usize = 10;
const AUTH_POLL: Duration = Duration::from_secs(5);
const AUTH_POLLS: u32 = 60;
const BUILT_IN_KEY: Option<&str> = option_env!("FEATHERTUNE_LASTFM_API_KEY");
const BUILT_IN_SECRET: Option<&str> = option_env!("FEATHERTUNE_LASTFM_API_SECRET");
/// This build carries API keys of its own.
pub const BUILT_IN_KEYS: bool = set(BUILT_IN_KEY) && set(BUILT_IN_SECRET);

const fn set(value: Option<&str>) -> bool {
    match value {
        Some(v) => !v.is_empty(),
        None => false,
    }
}

type OnChange = Arc<dyn Fn(bool) + Send + Sync>;

#[derive(Clone)]
pub struct LastFm(Arc<Inner>);

struct Inner {
    api_key: String,
    secret: String,
    agent: ureq::Agent,
    session: Mutex<Option<String>>,
    authenticating: AtomicBool,
    on_change: Mutex<Option<OnChange>>,
}

enum Error {
    Network(String),
    Api(i64, String),
}

impl Error {
    /// Worth trying again later; anything else means the request itself is wrong.
    fn transient(&self) -> bool {
        match self {
            Error::Network(_) => true,
            Error::Api(code, _) => matches!(code, 8 | 10 | 11 | 16 | 26 | 29),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Network(e) => write!(f, "network: {e}"),
            Error::Api(code, message) => write!(f, "api error {code}: {message}"),
        }
    }
}

/// Starts the scrobbler thread. `None` when the keys are not configured.
pub fn start(app: &AppHandle, settings: &Settings, profile: &Path) -> Option<LastFm> {
    let (api_key, secret) = match (settings.api_key.trim(), settings.api_secret.trim()) {
        (k, s) if !k.is_empty() && !s.is_empty() => (k, s),
        _ if BUILT_IN_KEYS => (BUILT_IN_KEY.unwrap_or_default().trim(), BUILT_IN_SECRET.unwrap_or_default().trim()),
        _ => return None,
    };
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
            )
            .user_agent(concat!("feathertune/", env!("CARGO_PKG_VERSION")))
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(15)))
            .build(),
    );
    let session = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).and_then(|e| e.get_password()).ok();
    let lastfm = LastFm(Arc::new(Inner {
        api_key: api_key.to_owned(),
        secret: secret.to_owned(),
        agent,
        session: Mutex::new(session),
        authenticating: AtomicBool::new(false),
        on_change: Mutex::new(None),
    }));
    let rx = app.state::<Player>().subscribe();
    let queue = Queue::load(profile.join("lastfm-queue.json"));
    let worker = lastfm.clone();
    std::thread::spawn(move || worker.run(rx, queue));
    Some(lastfm)
}

impl LastFm {
    pub fn connected(&self) -> bool {
        self.session().is_some()
    }

    /// Called with the new state whenever the session is created or lost.
    pub fn on_change(&self, f: impl Fn(bool) + Send + Sync + 'static) {
        *self.0.on_change.lock().unwrap() = Some(Arc::new(f));
    }

    /// Opens the authorization page in the browser and waits (in the background) for the approval.
    pub fn connect(&self, app: &AppHandle) {
        if self.0.authenticating.swap(true, Ordering::SeqCst) {
            return;
        }
        let (this, app) = (self.clone(), app.clone());
        std::thread::spawn(move || {
            this.authenticate(&app);
            this.0.authenticating.store(false, Ordering::SeqCst);
        });
    }

    /// Forgets the session key on this PC (it stays valid at Last.fm until revoked there).
    pub fn disconnect(&self) {
        self.set_session(None);
    }

    fn session(&self) -> Option<String> {
        self.0.session.lock().unwrap().clone()
    }

    fn set_session(&self, key: Option<String>) {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
            let result = match &key {
                Some(k) => entry.set_password(k),
                None => entry.delete_credential(),
            };
            if let Err(e) = result
                && !matches!(e, keyring::Error::NoEntry)
            {
                eprintln!("lastfm: credential store failed: {e}");
            }
        }
        let changed = {
            let mut session = self.0.session.lock().unwrap();
            let changed = session.is_some() != key.is_some();
            *session = key.clone();
            changed
        };
        // Called without the lock: the callback may wait for the main thread.
        let on_change = self.0.on_change.lock().unwrap().clone();
        if changed && let Some(f) = on_change {
            f(key.is_some());
        }
    }

    fn authenticate(&self, app: &AppHandle) {
        let token = match self.call("auth.getToken", Vec::new(), None) {
            Ok(v) => v["token"].as_str().map(str::to_owned),
            Err(e) => {
                eprintln!("lastfm: could not start sign-in: {e}");
                return;
            }
        };
        let Some(token) = token else { return };
        let Ok(url) =
            Url::parse_with_params(AUTH_URL, [("api_key", self.0.api_key.as_str()), ("token", token.as_str())])
        else {
            return;
        };
        if let Err(e) = app.opener().open_url(url.as_str(), None::<&str>) {
            eprintln!("lastfm: could not open the browser: {e}");
            return;
        }
        for _ in 0..AUTH_POLLS {
            std::thread::sleep(AUTH_POLL);
            match self.call("auth.getSession", vec![("token".into(), token.clone())], None) {
                Ok(v) => {
                    if let Some(key) = v["session"]["key"].as_str() {
                        self.set_session(Some(key.to_owned()));
                    }
                    return;
                }
                // 14: the user has not approved yet.
                Err(Error::Api(14, _)) | Err(Error::Network(_)) => {}
                Err(e) => {
                    eprintln!("lastfm: sign-in failed: {e}");
                    return;
                }
            }
        }
        eprintln!("lastfm: sign-in timed out");
    }

    /// One signed POST. `params` must not hold `method`, `api_key`, `sk`, `api_sig` or `format`.
    fn call(&self, method: &str, mut params: Vec<(String, String)>, session: Option<&str>) -> Result<Value, Error> {
        params.push(("method".into(), method.into()));
        params.push(("api_key".into(), self.0.api_key.clone()));
        if let Some(sk) = session {
            params.push(("sk".into(), sk.into()));
        }
        params.push(("api_sig".into(), sign(&params, &self.0.secret)));
        params.push(("format".into(), "json".into()));

        let mut response = self
            .0
            .agent
            .post(API)
            .send_form(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .map_err(|e| Error::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_string().map_err(|e| Error::Network(e.to_string()))?;
        let json: Value = serde_json::from_str(&body).map_err(|_| Error::Network(format!("HTTP {status}")))?;
        match json["error"].as_i64() {
            Some(code) => Err(Error::Api(code, json["message"].as_str().unwrap_or_default().to_owned())),
            None => Ok(json),
        }
    }

    fn update_now_playing(&self, t: &Tracking) {
        let Some(sk) = self.session().filter(|_| !t.artist.is_empty()) else { return };
        let mut params = vec![("artist".to_owned(), t.artist.clone()), ("track".into(), t.title.clone())];
        if !t.album.is_empty() {
            params.push(("album".into(), t.album.clone()));
        }
        if t.duration > 0.0 {
            params.push(("duration".into(), (t.duration.round() as u64).to_string()));
        }
        if let Err(e) = self.call("track.updateNowPlaying", params, Some(&sk)) {
            self.report(&e);
        }
    }

    /// A rejected session key means the user must sign in again.
    fn report(&self, e: &Error) {
        eprintln!("lastfm: {e}");
        if matches!(e, Error::Api(9, _)) {
            self.set_session(None);
        }
    }

    /// The worker: one thread, blocked on the next player event or on the next deadline (the
    /// scrobble point of the playing track, or the queue retry), never on a poll.
    fn run(&self, mut rx: Receiver<Event>, mut queue: Queue) {
        let mut tracking: Option<Tracking> = None;
        let mut retry = self.flush(&mut queue);
        loop {
            let deadline = [
                tracking.as_ref().and_then(Tracking::remaining),
                retry.map(|r| r.saturating_duration_since(Instant::now())),
            ]
            .into_iter()
            .flatten()
            .min();
            // The timeout is made inside the async block: its timer needs the runtime, which this
            // plain thread only enters in `block_on` (made outside, it panics and ends the thread).
            let received = match deadline {
                Some(d) => tauri::async_runtime::block_on(async { tokio::time::timeout(d, rx.recv()).await }).ok(),
                None => Some(tauri::async_runtime::block_on(rx.recv())),
            };
            match received {
                Some(Ok(event)) => self.handle(&event, &mut tracking),
                Some(Err(RecvError::Closed)) => break,
                Some(Err(RecvError::Lagged(_))) | None => {}
            }
            if let Some(t) = tracking.as_mut()
                && t.remaining() == Some(Duration::ZERO)
            {
                t.scrobbled = true;
                if self.session().is_some() {
                    queue.push(t.scrobble());
                    retry = self.flush(&mut queue);
                }
            }
            if retry.is_some_and(|r| r <= Instant::now()) {
                retry = self.flush(&mut queue);
            }
        }
    }

    fn handle(&self, event: &Event, tracking: &mut Option<Tracking>) {
        let state = event.state();
        let restart = match (event, tracking.as_ref()) {
            (Event::Track(_), _) | (_, None) => true,
            // Same track, but the position went back after the scrobble: it is playing again
            // (repeat one, or a seek to the start), which is a new listen.
            (Event::Progress(_), Some(t)) => {
                t.scrobbled && !state.paused && state.position_now() + 10.0 < t.last.position_now()
            }
            (Event::Playback(_), Some(_)) => false,
        };
        if restart {
            self.begin(state, tracking);
            return;
        }
        let Some(t) = tracking.as_mut() else { return };
        if matches!(event, Event::Playback(_)) {
            t.last = state.clone();
            if state.paused {
                t.pause();
            } else {
                t.since = Some(Instant::now());
                self.update_now_playing(t);
            }
        } else {
            t.update(state);
        }
    }

    fn begin(&self, state: &State, tracking: &mut Option<Tracking>) {
        let t = Tracking::new(state);
        if !state.paused {
            self.update_now_playing(&t);
        }
        *tracking = Some(t);
    }

    /// Sends the queue. Returns when to try again, `None` when there is nothing left to send.
    fn flush(&self, queue: &mut Queue) -> Option<Instant> {
        queue.prune();
        while !queue.items.is_empty() {
            let Some(sk) = self.session() else { break };
            let n = queue.items.len().min(BATCH);
            let mut params = Vec::new();
            for (i, s) in queue.items[..n].iter().enumerate() {
                params.push((format!("artist[{i}]"), s.artist.clone()));
                params.push((format!("track[{i}]"), s.title.clone()));
                params.push((format!("timestamp[{i}]"), s.timestamp.to_string()));
                if !s.album.is_empty() {
                    params.push((format!("album[{i}]"), s.album.clone()));
                }
                if s.duration > 0 {
                    params.push((format!("duration[{i}]"), s.duration.to_string()));
                }
            }
            match self.call("track.scrobble", params, Some(&sk)) {
                Ok(_) => queue.remove(n),
                Err(e) if e.transient() || matches!(e, Error::Api(9, _)) => {
                    self.report(&e);
                    break;
                }
                Err(e) => {
                    eprintln!("lastfm: dropping {n} scrobbles: {e}");
                    queue.remove(n);
                }
            }
        }
        (!queue.items.is_empty()).then(|| Instant::now() + RETRY)
    }
}

/// The track being played and how much of it has really been heard.
struct Tracking {
    artist: String,
    title: String,
    album: String,
    /// Seconds; 0 until the page reports it.
    duration: f64,
    /// Unix seconds when the track started.
    started: u64,
    /// Played time up to `since`; while `since` is set the time since then counts too.
    played: Duration,
    since: Option<Instant>,
    scrobbled: bool,
    /// The last state, to notice a position that went back.
    last: State,
}

impl Tracking {
    fn new(state: &State) -> Tracking {
        let t = &state.track;
        Tracking {
            artist: t.artist.clone(),
            title: t.title.clone(),
            album: t.album.clone(),
            duration: t.duration,
            started: unix_secs().saturating_sub(state.position_now() as u64),
            played: Duration::ZERO,
            since: (!state.paused).then(Instant::now),
            scrobbled: false,
            last: state.clone(),
        }
    }

    fn update(&mut self, state: &State) {
        if self.duration <= 0.0 {
            self.duration = state.track.duration;
        }
        if self.album.is_empty() {
            self.album = state.track.album.clone();
        }
        self.last = state.clone();
    }

    fn pause(&mut self) {
        if let Some(since) = self.since.take() {
            self.played += since.elapsed();
        }
    }

    /// Time still to play before the scrobble: `None` when it is not counting down (paused, already
    /// scrobbled, too short, length unknown, no artist), zero when it is due.
    fn remaining(&self) -> Option<Duration> {
        let since = self.since?;
        if self.scrobbled || self.artist.is_empty() || self.duration <= MIN_LENGTH {
            return None;
        }
        let needed = Duration::from_secs_f64((self.duration / 2.0).min(MAX_WAIT));
        Some(needed.saturating_sub(self.played + since.elapsed()))
    }

    fn scrobble(&self) -> Scrobble {
        Scrobble {
            artist: self.artist.clone(),
            title: self.title.clone(),
            album: self.album.clone(),
            duration: self.duration.round() as u32,
            timestamp: self.started,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Scrobble {
    artist: String,
    title: String,
    album: String,
    duration: u32,
    timestamp: u64,
}

/// Scrobbles waiting to be sent, kept on disk so a restart or a crash does not lose them.
struct Queue {
    path: PathBuf,
    items: Vec<Scrobble>,
}

impl Queue {
    fn load(path: PathBuf) -> Queue {
        let items = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Queue { path, items }
    }

    fn push(&mut self, s: Scrobble) {
        self.items.push(s);
        self.save();
    }

    fn remove(&mut self, n: usize) {
        self.items.drain(..n);
        self.save();
    }

    fn prune(&mut self) {
        let oldest = unix_secs().saturating_sub(MAX_AGE_SECS);
        let before = self.items.len();
        self.items.retain(|s| s.timestamp >= oldest);
        if self.items.len() != before {
            self.save();
        }
    }

    fn save(&self) {
        let result = if self.items.is_empty() {
            std::fs::remove_file(&self.path)
                .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
        } else {
            serde_json::to_vec(&self.items)
                .map_err(std::io::Error::other)
                .and_then(|data| std::fs::write(&self.path, data))
        };
        if let Err(e) = result {
            eprintln!("lastfm: could not save the queue: {e}");
        }
    }
}

/// `api_sig`: the parameters sorted by name, as `name` + `value` pairs, then the shared secret, MD5 in hex.
/// `format` is never part of it, so callers add it afterwards.
fn sign(params: &[(String, String)], secret: &str) -> String {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut text: String = sorted.iter().map(|(k, v)| format!("{k}{v}")).collect();
    text.push_str(secret);
    format!("{:x}", md5::compute(text))
}

fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Track;

    fn state(duration: f64, paused: bool) -> State {
        let track = Track {
            id: None,
            title: "Title".into(),
            artist: "Artist".into(),
            album: String::new(),
            artwork: None,
            duration,
        };
        State { track, position: 0.0, paused, at: Instant::now() }
    }

    fn needed(duration: f64) -> Option<u64> {
        Tracking::new(&state(duration, false)).remaining().map(|d| d.as_secs_f64().ceil() as u64)
    }

    #[test]
    fn signature_sorts_by_name_and_appends_the_secret() {
        let params = vec![("b".to_owned(), "2".to_owned()), ("a".to_owned(), "1".to_owned())];
        assert_eq!(sign(&params, "s"), "1d0396bcbc2c54e569e7af9cf9c4685e");
    }

    #[test]
    fn scrobbles_at_half_the_track_or_four_minutes() {
        assert_eq!(needed(200.0), Some(100));
        assert_eq!(needed(1000.0), Some(240));
    }

    #[test]
    fn short_or_unknown_tracks_are_never_due() {
        assert_eq!(needed(30.0), None);
        assert_eq!(needed(0.0), None);
    }

    #[test]
    fn only_played_time_counts() {
        let mut t = Tracking::new(&state(200.0, false));
        t.played = Duration::from_secs(100);
        assert_eq!(t.remaining(), Some(Duration::ZERO));
        t.pause();
        assert_eq!(t.remaining(), None);
        let mut paused = Tracking::new(&state(200.0, true));
        paused.played = Duration::from_secs(60);
        assert_eq!(paused.remaining(), None);
    }

    #[test]
    fn old_scrobbles_are_pruned() {
        let item = |timestamp| Scrobble {
            artist: "a".into(),
            title: "t".into(),
            album: String::new(),
            duration: 0,
            timestamp,
        };
        let mut queue = Queue { path: std::env::temp_dir().join("feathertune-queue-test.json"), items: Vec::new() };
        queue.items = vec![item(1), item(unix_secs())];
        queue.prune();
        assert_eq!(queue.items.len(), 1);
        let _ = std::fs::remove_file(&queue.path);
    }
}
