//! Our entry in the Windows media flyout and the keyboard media keys (SystemMediaTransportControls).
//! WebView2's own entry shows up as "msedgewebview2.exe", so Chromium's is turned off
//! (`HardwareMediaKeyHandling` in BROWSER_ARGS) and this one, tied to the main window, is fed by
//! the `Player` state. The buttons reuse the tray commands.

use tauri::{AppHandle, Manager, Window};
use tokio::sync::broadcast::error::RecvError;
use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::{ISystemMediaTransportControlsInterop, RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{HSTRING, Ref, Result, factory};

use crate::player::{Player, State};

/// Same as `identifier` in tauri.conf.json.
const APP_ID: &str = "dev.feathertune.desktop";

/// Gives the process an AppUserModelID and makes sure a Start menu shortcut carries it: the media
/// flyout takes the app name from that shortcut, and shows "Unknown app" without it. Must run
/// before any window exists. The shortcut (Start Menu\Programs\feathertune.lnk under %APPDATA%) is
/// only created when missing, so one made by the installer is kept; the uninstaller removes it.
pub fn set_app_id() {
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_ID)) } {
        eprintln!("SetCurrentProcessExplicitAppUserModelID failed: {e}");
    }
    std::thread::spawn(|| {
        if let Err(e) = ensure_shortcut() {
            eprintln!("could not create the Start menu shortcut: {e}");
        }
    });
}

fn ensure_shortcut() -> Result<()> {
    use std::mem::ManuallyDrop;
    use std::path::PathBuf;

    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::{
        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{Interface, PWSTR};

    let (Some(appdata), Ok(exe)) = (std::env::var_os("APPDATA"), std::env::current_exe()) else {
        return Ok(());
    };
    let lnk = PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\feathertune.lnk");
    if lnk.exists() {
        return Ok(());
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let exe = HSTRING::from(exe.as_path());
        link.SetPath(&exe)?;
        link.SetIconLocation(&exe, 0)?;
        let mut id: Vec<u16> = APP_ID.encode_utf16().chain([0]).collect();
        let value = PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_LPWSTR,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 { pwszVal: PWSTR(id.as_mut_ptr()) },
                }),
            },
        };
        let store: IPropertyStore = link.cast()?;
        store.SetValue(&PKEY_AppUserModel_ID, &value)?;
        store.Commit()?;
        link.cast::<IPersistFile>()?.Save(&HSTRING::from(lnk.as_path()), true)
    }
}

pub fn start(app: &AppHandle, window: &Window) -> Result<()> {
    let hwnd = window.hwnd().map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
    let interop = factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
    let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd.0))? };
    smtc.SetIsEnabled(true)?;
    smtc.SetIsPlayEnabled(true)?;
    smtc.SetIsPauseEnabled(true)?;
    smtc.SetIsNextEnabled(true)?;
    smtc.SetIsPreviousEnabled(true)?;

    let handle = app.clone();
    smtc.ButtonPressed(&TypedEventHandler::new(
        move |_, args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
            let command = match args.ok()?.Button()? {
                SystemMediaTransportControlsButton::Play => "play",
                SystemMediaTransportControlsButton::Pause => "pause",
                SystemMediaTransportControlsButton::Next => "next",
                SystemMediaTransportControlsButton::Previous => "previous",
                _ => return Ok(()),
            };
            crate::player(&handle, command);
            Ok(())
        },
    ))?;

    let mut rx = app.state::<Player>().subscribe();
    std::thread::spawn(move || {
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let mut shown = None;
        loop {
            match rx.blocking_recv() {
                Ok(event) => {
                    if let Err(e) = show(&smtc, event.state(), &mut shown) {
                        eprintln!("smtc update failed: {e}");
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
    Ok(())
}

/// `shown` is the (title, artist, album, artwork) on the flyout, so it is only rewritten when it changes.
fn show(smtc: &SystemMediaTransportControls, s: &State, shown: &mut Option<[String; 4]>) -> Result<()> {
    let t = &s.track;
    let display = [t.title.clone(), t.artist.clone(), t.album.clone(), t.artwork.clone().unwrap_or_default()];
    if shown.as_ref() != Some(&display) {
        let du = smtc.DisplayUpdater()?;
        du.ClearAll()?;
        du.SetType(MediaPlaybackType::Music)?;
        let music = du.MusicProperties()?;
        music.SetTitle(&HSTRING::from(&t.title))?;
        music.SetArtist(&HSTRING::from(&t.artist))?;
        music.SetAlbumTitle(&HSTRING::from(&t.album))?;
        if let Some(url) = &t.artwork {
            du.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&Uri::CreateUri(&HSTRING::from(url))?)?)?;
        }
        du.Update()?;
        *shown = Some(display);
    }
    smtc.SetPlaybackStatus(if s.paused { MediaPlaybackStatus::Paused } else { MediaPlaybackStatus::Playing })?;

    let span = |secs: f64| TimeSpan { Duration: (secs * 1e7) as i64 };
    let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
    timeline.SetStartTime(span(0.0))?;
    timeline.SetMinSeekTime(span(0.0))?;
    timeline.SetEndTime(span(t.duration))?;
    timeline.SetMaxSeekTime(span(t.duration))?;
    timeline.SetPosition(span(s.position_now().min(t.duration)))?;
    smtc.UpdateTimelineProperties(&timeline)
}
