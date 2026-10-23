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
//!   A change of `volume`, `muted` or `ad` (the ad flag of the page) is an `Event::Progress`.
//!
//!   The receiver is a `tokio::sync::broadcast` one: use `recv().await` in async code or
//!   `blocking_recv()` on a plain thread. A slow consumer gets `RecvError::Lagged` and should just
//!   keep receiving, since the next event has the full state again.
//!
//! - `Player::queue() -> Queue` and `Player::subscribe_queue() -> broadcast::Receiver<Queue>`: the last
//!   play queue the page reported and the index of the playing track in it, for any playlist, radio or
//!   autoplay. A separate channel, so `Event` consumers are not affected; it fires when the items or the
//!   index change. Each `Queue` is the whole queue (the same lag rule applies).
//!
//! Everything in `State` comes from the page and was validated here: strings are length-capped,
//! numbers are finite and in range, and `artwork` is an https URL on one of the image hosts below. It is
//! still display data only: never use it to build commands, paths or markup.

use std::sync::Mutex;
use std::time::Instant;

use serde::Deserialize;
use tauri::Url;
use tokio::sync::broadcast;

/// Raw payload limit, before parsing (a queue of 200 tracks fits).
const MAX_PAYLOAD: usize = 96 * 1024;
const MAX_QUEUE: usize = 200;
const MAX_ITEM_TEXT: usize = 200;
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

/// A track of the play queue (`player_state`'s `next` and `queue`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueItem {
    pub id: String,
    pub title: String,
    pub artist: String,
}

/// Where the track plays from, for the mixes (mixes.rs): the playlist id (`list=` of the URL), the next
/// track and, when the queue changed, the queue. Data only, like `State`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    pub list: Option<String>,
    pub next: Option<QueueItem>,
    pub queue: Option<Vec<QueueItem>>,
}

/// The play queue as the page last reported it (`player_state`'s `queue` and `current`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queue {
    pub items: Vec<QueueItem>,
    /// Index in `items` of the track playing, when known.
    pub current: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct State {
    pub track: Track,
    /// Seconds from the start of the track, as read at `at`.
    pub position: f64,
    pub paused: bool,
    /// The player's own volume, 0–100.
    pub volume: u8,
    pub muted: bool,
    /// An ad is showing.
    pub ad: bool,
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
    queue: Mutex<Queue>,
    queue_tx: broadcast::Sender<Queue>,
}

impl Player {
    pub fn new() -> Player {
        Player {
            current: Mutex::new(None),
            tx: broadcast::channel(32).0,
            queue: Mutex::new(Queue::default()),
            queue_tx: broadcast::channel(8).0,
        }
    }

    pub fn current(&self) -> Option<State> {
        self.current.lock().unwrap().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// The last queue reported (empty until the page reports one).
    pub fn queue(&self) -> Queue {
        self.queue.lock().unwrap().clone()
    }

    pub fn subscribe_queue(&self) -> broadcast::Receiver<Queue> {
        self.queue_tx.subscribe()
    }

    /// `items` is `Some` only when the page sent the queue (it changed); `current` comes with every payload
    /// the page could read the site's state for. Neither: nothing to learn. An index outside the queue is
    /// dropped.
    fn update_queue(&self, items: Option<Vec<QueueItem>>, current: Option<usize>) {
        if items.is_none() && current.is_none() {
            return;
        }
        let mut queue = self.queue.lock().unwrap();
        let mut changed = false;
        if let Some(items) = items
            && queue.items != items
        {
            queue.items = items;
            changed = true;
        }
        let current = current.filter(|&i| i < queue.items.len());
        if queue.current != current {
            queue.current = current;
            changed = true;
        }
        if changed {
            let snapshot = queue.clone();
            drop(queue);
            let _ = self.queue_tx.send(snapshot);
        }
    }

    fn update(&self, new: State) {
        let mut current = self.current.lock().unwrap();
        let event = match current.as_ref() {
            Some(old)
                if old.track == new.track
                    && old.paused == new.paused
                    && old.position == new.position
                    && old.volume == new.volume
                    && old.muted == new.muted
                    && old.ad == new.ad =>
            {
                return;
            }
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
pub fn player_state(state: String, player: tauri::State<'_, Player>, app: tauri::AppHandle) {
    match parse(&state) {
        Some((s, context, current)) => {
            player.update(s);
            player.update_queue(context.queue.clone(), current);
            crate::mixes::observe(&app, context);
        }
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
    volume: f64,
    muted: bool,
    ad: bool,
    #[serde(default)]
    list: Option<String>,
    #[serde(default)]
    next: Option<QueueItem>,
    #[serde(default)]
    queue: Option<Vec<QueueItem>>,
    /// Index of the playing track in the site's queue (in `queue` when that is sent too).
    #[serde(default)]
    current: Option<usize>,
}

/// The state, where it plays from and the index of the playing track in the queue.
fn parse(json: &str) -> Option<(State, Context, Option<usize>)> {
    if json.len() > MAX_PAYLOAD {
        return None;
    }
    let p: Payload = serde_json::from_str(json).ok()?;
    let text =
        |s: String| (s.chars().count() <= MAX_TEXT && !s.chars().any(char::is_control)).then(|| s.trim().to_owned());
    let seconds = |x: f64| (x.is_finite() && (0.0..=MAX_SECONDS).contains(&x)).then_some(x);
    let id = match p.id {
        Some(id) if valid_id(&id) => Some(id),
        Some(_) => return None,
        None => None,
    };
    let title = text(p.title).filter(|t| !t.is_empty())?;
    let artwork = match p.artwork {
        Some(a) => Some(artwork(&a)?),
        None => None,
    };
    let item = |i: QueueItem| {
        let short = |s: String| {
            (s.chars().count() <= MAX_ITEM_TEXT && !s.chars().any(char::is_control)).then(|| s.trim().to_owned())
        };
        Some(QueueItem { title: short(i.title)?, artist: short(i.artist)?, id: valid_id(&i.id).then_some(i.id)? })
    };
    let list = match p.list {
        Some(l) if valid_list(&l) => Some(l),
        Some(_) => return None,
        None => None,
    };
    let next = match p.next {
        Some(n) => Some(item(n)?),
        None => None,
    };
    let queue = match p.queue {
        Some(q) if q.len() <= MAX_QUEUE => Some(q.into_iter().map(item).collect::<Option<Vec<_>>>()?),
        Some(_) => return None,
        None => None,
    };
    let current = match (p.current, &queue) {
        (Some(i), Some(q)) if i < q.len() => Some(i),
        (Some(i), None) if i < MAX_QUEUE => Some(i),
        (Some(_), _) => return None,
        (None, _) => None,
    };
    if !(p.volume.is_finite() && (0.0..=100.0).contains(&p.volume)) {
        return None;
    }
    let state = State {
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
        volume: p.volume.round() as u8,
        muted: p.muted,
        ad: p.ad,
        at: Instant::now(),
    };
    Some((state, Context { list, next, queue }, current))
}

pub fn valid_id(id: &str) -> bool {
    crate::audio_facts::valid_id(id)
}

/// A playlist id: the `list=` of a URL.
pub fn valid_list(list: &str) -> bool {
    (1..=64).contains(&list.len()) && list.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The URL if it is an https URL on one of the image hosts (the mini player checks downloads with it too).
pub(crate) fn artwork(url: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(extra: &str) -> String {
        format!(
            r#"{{"id":"dQw4w9WgXcQ","title":"T","artist":"A","album":"","artwork":null,"duration":200,"position":3,"paused":false,"volume":40,"muted":false,"ad":false{extra}}}"#
        )
    }

    fn item(id: char) -> QueueItem {
        QueueItem { id: id.to_string().repeat(11), title: "T".into(), artist: "A".into() }
    }

    #[test]
    fn volume_muted_and_ad_are_read_and_bounded() {
        let (s, _, _) = parse(&payload("")).unwrap();
        assert_eq!((s.volume, s.muted, s.ad), (40, false, false));
        let json =
            payload("").replace(r#""volume":40,"muted":false,"ad":false"#, r#""volume":99.6,"muted":true,"ad":true"#);
        let (s, _, _) = parse(&json).unwrap();
        assert_eq!((s.volume, s.muted, s.ad), (100, true, true));
        for (from, to) in [
            (r#""volume":40"#, r#""volume":-1"#),
            (r#""volume":40"#, r#""volume":100.5"#),
            (r#""volume":40"#, r#""volume":"40""#),
            (r#""volume":40"#, r#""volume":null"#),
            (r#""muted":false"#, r#""muted":1"#),
            (r#""ad":false"#, r#""ad":"yes""#),
            (r#","ad":false"#, ""),
            (r#""volume":40,"#, ""),
            (r#""muted":false,"#, ""),
        ] {
            assert!(parse(&payload("").replacen(from, to, 1)).is_none(), "{to}");
        }
        assert!(parse(&payload(r#","extra":1"#)).is_none());
    }

    #[test]
    fn current_is_checked_against_the_queue() {
        let q =
            r#""queue":[{"id":"aaaaaaaaaaa","title":"x","artist":"y"},{"id":"bbbbbbbbbbb","title":"x","artist":"y"}]"#;
        assert_eq!(parse(&payload(&format!(r#",{q},"current":1"#))).unwrap().2, Some(1));
        assert_eq!(parse(&payload(&format!(r#",{q}"#))).unwrap().2, None);
        assert_eq!(parse(&payload(r#","current":5"#)).unwrap().2, Some(5));
        for bad in [
            format!(r#",{q},"current":2"#),
            format!(r#","current":{MAX_QUEUE}"#),
            r#","current":-1"#.to_owned(),
            r#","current":1.5"#.to_owned(),
        ] {
            assert!(parse(&payload(&bad)).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_queue_without_a_playlist_reaches_the_player_but_not_the_mixes_context() {
        let json = payload(r#","queue":[{"id":"aaaaaaaaaaa","title":"x","artist":"y"}],"current":0"#);
        let (_, c, current) = parse(&json).unwrap();
        assert_eq!(c.list, None);
        assert_eq!(c.queue.map(|q| q.len()), Some(1));
        assert_eq!(current, Some(0));
    }

    #[test]
    fn the_player_keeps_the_queue_and_tells_when_it_changes() {
        let player = Player::new();
        let mut rx = player.subscribe_queue();
        assert_eq!(player.queue(), Queue::default());
        // Nothing read from the site: nothing to learn.
        player.update_queue(None, None);
        assert!(rx.try_recv().is_err());

        player.update_queue(Some(vec![item('a'), item('b')]), Some(0));
        let expected = Queue { items: vec![item('a'), item('b')], current: Some(0) };
        assert_eq!(rx.try_recv().unwrap(), expected);
        assert_eq!(player.queue(), expected);

        // Same data again: no event.
        player.update_queue(Some(vec![item('a'), item('b')]), Some(0));
        player.update_queue(None, Some(0));
        assert!(rx.try_recv().is_err());

        // Only the index moved (the queue was not sent): the items stay.
        player.update_queue(None, Some(1));
        assert_eq!(rx.try_recv().unwrap(), Queue { items: vec![item('a'), item('b')], current: Some(1) });

        // An index outside the queue is dropped, not stored.
        player.update_queue(None, Some(2));
        assert_eq!(rx.try_recv().unwrap(), Queue { items: vec![item('a'), item('b')], current: None });
        player.update_queue(Some(vec![item('c')]), Some(1));
        assert_eq!(rx.try_recv().unwrap(), Queue { items: vec![item('c')], current: None });
    }

    #[test]
    fn volume_muted_and_ad_changes_are_progress_events() {
        let player = Player::new();
        let mut rx = player.subscribe();
        let (base, _, _) = parse(&payload("")).unwrap();
        player.update(base.clone());
        assert!(matches!(rx.try_recv(), Ok(Event::Track(_))));
        player.update(State { at: Instant::now(), ..base.clone() });
        assert!(rx.try_recv().is_err());
        for changed in [
            State { volume: 41, ..base.clone() },
            State { muted: true, ..base.clone() },
            State { ad: true, ..base.clone() },
        ] {
            player.update(changed);
            assert!(matches!(rx.try_recv(), Ok(Event::Progress(_))));
        }
    }

    #[test]
    fn list_next_and_queue_are_validated() {
        let (_, c, _) = parse(&payload("")).unwrap();
        assert_eq!(c, Context::default());
        let item = r#"{"id":"aaaaaaaaaaa","title":"Next","artist":"Someone"}"#;
        let (_, c, _) = parse(&payload(&format!(r#","list":"PLx_1-2","next":{item},"queue":[{item}]"#))).unwrap();
        assert_eq!(c.list.as_deref(), Some("PLx_1-2"));
        assert_eq!(c.next.as_ref().map(|n| n.title.as_str()), Some("Next"));
        assert_eq!(c.queue.map(|q| q.len()), Some(1));
        for bad in [
            r#","list":"../etc""#.to_owned(),
            format!(r#","list":"{}""#, "a".repeat(65)),
            r#","next":{"id":"short","title":"","artist":""}"#.to_owned(),
            r#","next":{"id":"aaaaaaaaaaa","title":"x","artist":"y","extra":1}"#.to_owned(),
            format!(r#","queue":[{}]"#, vec![item; MAX_QUEUE + 1].join(",")),
        ] {
            assert!(parse(&payload(&bad)).is_none(), "{bad}");
        }
    }
}
