//! Discord Rich Presence: "Listening to YouTube Music" with the title, the artist, the album on the
//! artwork, the site's icon, a time bar, links on the title, the artist and the artwork, and a button
//! to open the track; cleared on pause. The application id is `[discord] client_id`, else the one a CI
//! release is built with (`FEATHERTUNE_DISCORD_CLIENT_ID`, a repository secret); off with neither. Discord shows the
//! activity name only where it honors `name`; elsewhere it is the name of the Discord application.
//!
//! Failures are silent: if Discord is not running the connection is simply tried again at the next
//! player event (a track change, a pause or the page's 15 s position refresh), so there is no timer.

use std::time::{SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, ActivityType, Assets, Button, StatusDisplayType, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use tauri::{AppHandle, Manager, Url};
use tokio::sync::broadcast::error::RecvError;

use crate::player::{Player, State};

/// The position refresh arrives every 15 s; only a drift beyond this (a seek) is worth a new update.
const MAX_DRIFT_MS: i64 = 2_000;
const NAME: &str = "YouTube Music";
const SITE: &str = "https://music.youtube.com";
const ICON: &str = "https://music.youtube.com/img/favicon_144.png";
/// Discord rejects longer URLs.
const MAX_URL: usize = 256;
const BUILT_IN_ID: Option<&str> = option_env!("FEATHERTUNE_DISCORD_CLIENT_ID");
/// This build carries an application id of its own.
pub const BUILT_IN: bool = matches!(BUILT_IN_ID, Some(id) if !id.is_empty());

pub fn start(app: &AppHandle, client_id: &str) {
    let client_id = match client_id.trim() {
        "" => BUILT_IN_ID.unwrap_or_default().trim(),
        id => id,
    };
    if client_id.is_empty() {
        return;
    }
    let mut rx = app.state::<Player>().subscribe();
    let mut presence = Presence { client: DiscordIpcClient::new(client_id), connected: false, sent: None };
    std::thread::spawn(move || {
        loop {
            match rx.blocking_recv() {
                Ok(event) => presence.update(event.state()),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
        presence.clear();
    });
}

struct Presence {
    client: DiscordIpcClient,
    connected: bool,
    /// The track and start time of the activity Discord is showing.
    sent: Option<(String, i64)>,
}

impl Presence {
    fn update(&mut self, state: &State) {
        if state.paused {
            self.clear();
            return;
        }
        let t = &state.track;
        let key = format!("{}\u{0}{}\u{0}{}", t.title, t.artist, t.duration);
        let start = now_ms() - (state.position_now() * 1000.0) as i64;
        if self.sent.as_ref().is_some_and(|(k, s)| *k == key && (s - start).abs() < MAX_DRIFT_MS) {
            return;
        }
        if !self.connect() {
            return;
        }
        let mut times = Timestamps::new().start(start);
        if t.duration > 0.0 {
            times = times.end(start + (t.duration * 1000.0) as i64);
        }
        let details = fit(&t.title);
        let artist = fit(&t.artist);
        let album = fit(&t.album).or_else(|| details.clone());
        let watch = t.id.as_ref().map(|id| format!("{SITE}/watch?v={id}"));
        let search = Url::parse_with_params(&format!("{SITE}/search"), [("q", t.artist.as_str())])
            .ok()
            .map(String::from)
            .filter(|u| u.len() <= MAX_URL && artist.is_some());
        let mut activity = Activity::new()
            .name(NAME)
            .activity_type(ActivityType::Listening)
            .status_display_type(StatusDisplayType::Name)
            .timestamps(times);
        if let Some(d) = &details {
            activity = activity.details(d.as_str());
            if let Some(w) = &watch {
                activity = activity.details_url(w.as_str());
            }
        }
        if let Some(a) = &artist {
            activity = activity.state(a.as_str());
            if let Some(s) = &search {
                activity = activity.state_url(s.as_str());
            }
        }
        let mut assets = Assets::new().small_image(ICON).small_text(NAME);
        if let Some(art) = t.artwork.as_deref().filter(|a| a.len() <= MAX_URL) {
            assets = assets.large_image(art);
            if let Some(a) = &album {
                assets = assets.large_text(a.as_str());
            }
            if let Some(w) = &watch {
                assets = assets.large_url(w.as_str());
            }
        }
        activity = activity.assets(assets);
        if let Some(w) = &watch {
            activity = activity.buttons(vec![Button::new("Listen on YouTube Music", w.as_str())]);
        }
        match self.client.set_activity(activity) {
            Ok(()) => self.sent = Some((key, start)),
            Err(_) => self.disconnect(),
        }
    }

    fn clear(&mut self) {
        if self.connected && self.client.clear_activity().is_err() {
            self.disconnect();
        }
        self.sent = None;
    }

    fn connect(&mut self) -> bool {
        if !self.connected {
            self.connected = self.client.connect().is_ok();
        }
        self.connected
    }

    fn disconnect(&mut self) {
        let _ = self.client.close();
        self.connected = false;
        self.sent = None;
    }
}

/// Discord accepts 2 to 128 characters in these fields.
fn fit(text: &str) -> Option<String> {
    (text.chars().count() >= 2).then(|| text.chars().take(128).collect())
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}
