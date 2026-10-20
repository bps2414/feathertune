//! Track-change toast (plugin `track_notifications`, checked on every track so the switch is live),
//! only while the main window is hidden in the tray: with the window open the player bar already
//! shows the track, and the mini player shows it too. Shown from Rust with the same crate the notification plugin
//! uses, because the plugin injects a script that replaces `window.Notification` in the page.
//! The toast carries the app's AppUserModelID, so it shows under the app's name (see `smtc::set_app_id`).

use tauri::{AppHandle, Manager};
use tauri_winrt_notification::Toast;
use tokio::sync::broadcast::error::RecvError;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

use crate::player::{Event, Player};
use crate::settings::Live;

pub fn start(app: &AppHandle) {
    let mut rx = app.state::<Player>().subscribe();
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let app_id = app.config().identifier.clone();
        loop {
            match rx.blocking_recv() {
                Ok(Event::Track(state)) if !state.paused => {
                    // The switch is live (settings window); the mini player already shows the track.
                    let hidden = app.get_window("main").is_some_and(|w| !w.is_visible().unwrap_or(true));
                    if !hidden || crate::miniplayer::is_open() || !app.state::<Live>().plugin("track_notifications") {
                        continue;
                    }
                    let track = &state.track;
                    let mut toast = Toast::new(&app_id).title(&track.title).sound(None);
                    if !track.artist.is_empty() {
                        toast = toast.text1(&track.artist);
                    }
                    if !track.album.is_empty() {
                        toast = toast.text2(&track.album);
                    }
                    if let Err(e) = toast.show() {
                        eprintln!("track notification failed: {e}");
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
}
