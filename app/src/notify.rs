//! Track-change toast, only while the main window is hidden (in the tray): with the window open the
//! player bar already shows the track. Shown from Rust with the same crate the notification plugin
//! uses, because the plugin injects a script that replaces `window.Notification` in the page.
//! The toast carries the app's AppUserModelID, so it shows under the app's name (see `smtc::set_app_id`).

use tauri::{AppHandle, Manager};
use tauri_winrt_notification::Toast;
use tokio::sync::broadcast::error::RecvError;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

use crate::player::{Event, Player};

pub fn start(app: &AppHandle) {
    let mut rx = app.state::<Player>().subscribe();
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let app_id = app.config().identifier.clone();
        loop {
            match rx.blocking_recv() {
                Ok(Event::Track(state)) if !state.paused => {
                    let hidden = app.get_webview_window("main").is_some_and(|w| !w.is_visible().unwrap_or(true));
                    if !hidden {
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
