//! The sleep timer (plugin `sleep_timer`): pauses the music after 15 to 60 minutes, or just before the
//! current track ends (the next one that starts, when none plays yet), chosen from the tray menu (a flat group of checked items in both menus). Kept in memory
//! only: forgotten when the app closes. One thread runs only while a timer is set, and it pauses through
//! the `controls` page module (`player(app, "pause")`), like the tray's own buttons. A timer that runs out
//! during an ad waits for the ad to end (the pause would be refused, and the ad is the site's).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::i18n::{t, tf};
use crate::player::{Player, State};
use crate::settings::Live;

/// The tray items: their ids and minutes (0: at the end of this track).
pub const ITEMS: &[(&str, u64)] =
    &[("sleep_15", 15), ("sleep_30", 30), ("sleep_45", 45), ("sleep_60", 60), ("sleep_track", 0)];
pub const OFF_ID: &str = "sleep_off";
const TICK: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, PartialEq)]
enum Timer {
    Off,
    /// The item's minutes, and when it pauses.
    At(u64, Instant),
    EndOfTrack,
}

/// The timer, and a count that tells an old thread to stop.
static STATE: Mutex<(Timer, u64)> = Mutex::new((Timer::Off, 0));

/// Whether the timer can be used now (it pauses through Player controls).
pub fn available(app: &AppHandle) -> bool {
    let live = app.state::<Live>();
    live.plugin("sleep_timer") && live.plugin("controls")
}

/// Whether the tray item `id` is the timer set now.
pub fn checked(id: &str) -> bool {
    let timer = STATE.lock().unwrap().0;
    match timer {
        Timer::Off => id == OFF_ID,
        Timer::At(minutes, _) => ITEMS.iter().any(|&(i, m)| i == id && m == minutes),
        Timer::EndOfTrack => id == "sleep_track",
    }
}

/// The tray item's text; with `left`, the minutes left on the one set now.
pub fn label(id: &str, left: bool) -> String {
    let minutes = ITEMS.iter().find(|&&(i, _)| i == id).map(|&(_, m)| m);
    match minutes {
        None => t("Sleep timer off").into(),
        Some(0) => t("Pause at the end of this track").into(),
        Some(m) => {
            let text = tf("Pause in {} min", &[&m.to_string()]);
            match STATE.lock().unwrap().0 {
                Timer::At(set, at) if left && set == m => {
                    let rest = at.saturating_duration_since(Instant::now()).as_secs().div_ceil(60);
                    tf("{} ({} min left)", &[&text, &rest.to_string()])
                }
                _ => text,
            }
        }
    }
}

/// A tray item was chosen: sets the timer (or turns it off).
pub fn choose(app: &AppHandle, id: &str) {
    let timer = match ITEMS.iter().find(|&&(i, _)| i == id) {
        _ if !available(app) => Timer::Off,
        Some(&(_, 0)) => Timer::EndOfTrack,
        Some(&(_, m)) => Timer::At(m, Instant::now() + Duration::from_secs(m * 60)),
        None => Timer::Off,
    };
    let generation = {
        let mut state = STATE.lock().unwrap();
        state.1 += 1;
        state.0 = timer;
        state.1
    };
    crate::sync_sleep_items(app);
    if timer == Timer::Off {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || run(&app, timer, generation));
}

/// "End of this track" pauses when less than this is left of it (seconds).
const END_MARGIN: f64 = 0.4;

/// For "end of this track": whether to pause now, and how long to wait before the next check. `target` is the
/// track it waits for: `None` until a track plays (it then becomes that one). It pauses when less than
/// `END_MARGIN` is left of the target while it plays, or, as a fallback, once another track plays. An ad is
/// never a track here.
fn end_of_track(target: &mut Option<String>, now: Option<&State>) -> (bool, Duration) {
    let Some(s) = now.filter(|s| !s.ad) else { return (false, TICK) };
    let Some(id) = s.track.id.as_deref() else { return (false, TICK) };
    match target {
        None => *target = Some(id.to_owned()),
        Some(t) if t != id => return (true, TICK),
        Some(_) => {}
    }
    if s.paused || s.track.duration <= 0.0 {
        return (false, TICK);
    }
    let left = s.track.duration - s.position_now();
    if left < END_MARGIN {
        return (true, TICK);
    }
    // Wake up right at the margin when it comes before the next tick.
    (false, TICK.min(Duration::from_secs_f64(left - END_MARGIN)).max(Duration::from_millis(20)))
}

fn run(app: &AppHandle, timer: Timer, generation: u64) {
    let current = || app.state::<Player>().current();
    let mut target = current().filter(|s| !s.ad).and_then(|s| s.track.id);
    let mut wait = TICK;
    loop {
        std::thread::sleep(wait);
        if STATE.lock().unwrap().1 != generation {
            return;
        }
        let now = current();
        let due = match timer {
            Timer::At(_, at) => Instant::now() >= at,
            Timer::EndOfTrack => {
                let (due, next) = end_of_track(&mut target, now.as_ref());
                wait = next;
                due
            }
            Timer::Off => return,
        };
        if due && !now.is_some_and(|s| s.ad) {
            break;
        }
    }
    {
        let mut state = STATE.lock().unwrap();
        if state.1 != generation {
            return;
        }
        state.0 = Timer::Off;
    }
    crate::player(app, "pause");
    crate::sync_sleep_items(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Track;

    fn state(id: &str, position: f64, paused: bool, ad: bool) -> State {
        let track = Track {
            id: Some(id.into()),
            title: "T".into(),
            artist: "A".into(),
            album: String::new(),
            artwork: None,
            duration: 200.0,
        };
        State { track, position, paused, volume: 50, muted: false, ad, at: Instant::now() }
    }

    #[test]
    fn end_of_track_pauses_just_before_the_end_of_the_track_it_waits_for() {
        // Nothing plays yet: it waits, then takes the first track that plays (an ad is not one).
        let mut target = None;
        assert!(!end_of_track(&mut target, None).0);
        assert!(!end_of_track(&mut target, Some(&state("ad", 199.9, false, true))).0);
        assert_eq!(target, None);
        assert!(!end_of_track(&mut target, Some(&state("a", 10.0, false, false))).0);
        assert_eq!(target.as_deref(), Some("a"));
        // Near its end: due; paused near its end: not yet.
        assert!(end_of_track(&mut target, Some(&state("a", 199.8, false, false))).0);
        assert!(!end_of_track(&mut target, Some(&state("a", 199.8, true, false))).0);
        // The wait never overshoots the margin.
        let (_, wait) = end_of_track(&mut target, Some(&state("a", 199.0, false, false)));
        assert!(wait <= Duration::from_millis(610));
        // Fallback: another track plays.
        assert!(end_of_track(&mut target, Some(&state("b", 0.0, false, false))).0);
    }
}
