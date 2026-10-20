//! Taskbar (plugin `taskbar_controls`): previous / play-pause / next buttons on the main window's
//! thumbnail, and the track's progress on its taskbar button (yellow while paused).
//!
//! - `ITaskbarList3`, on the main thread, from a subclass of the main window. The buttons can only be
//!   added once the taskbar button exists, which Windows announces with the registered
//!   "TaskbarButtonCreated" message; it comes again whenever the button is made again (the window shown
//!   from the tray, Explorer restarted), and the buttons are added again then.
//! - The icons are the mini player's glyphs, drawn white with Direct2D (`gfx::Offscreen`) at the small
//!   icon size of the window's DPI, once.
//! - A worker thread forwards the player events and, while a track plays and the window is shown,
//!   wakes once per 1% of the track (between 1 and 5 s) to move the progress. Nothing runs while
//!   paused, stopped or in the tray.
//! - A click runs the same `controls` command as the tray.

use std::time::Duration;

use tauri::{AppHandle, Manager, Window};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::DirectWrite::{DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED,
    THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_ENABLED, THBN_CLICKED, THUMBBUTTON, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateIconIndirect, DestroyIcon, HICON, ICONINFO, IsWindowVisible, MSGFLT_ALLOW,
    PostMessageW, RegisterWindowMessageW, SM_CXSMICON, WM_APP, WM_COMMAND, WM_NCDESTROY,
};
use windows::core::{Result, w};

use crate::gfx::{self, ICON_FONT, rect, rgb};
use crate::player::{Event, Player};

const SUBCLASS_ID: usize = 0xfeab;
/// Posted by the worker: the player changed, or the progress moved.
const WM_UPDATE: u32 = WM_APP + 4;
/// Thumbnail button ids, in order.
const COMMANDS: [&str; 3] = ["previous", "playPause", "next"];
const PREVIOUS: &str = "\u{e892}";
const PLAY: &str = "\u{e768}";
const PAUSE: &str = "\u{e769}";
const NEXT: &str = "\u{e893}";

struct Taskbar {
    app: AppHandle,
    /// The registered "TaskbarButtonCreated" message.
    created: u32,
    list: Option<ITaskbarList3>,
    /// The buttons are on the current taskbar button.
    added: bool,
    /// previous, play, pause, next; made on first use.
    icons: Option<[HICON; 4]>,
    /// Whether the middle button shows pause (something plays).
    shows_pause: bool,
}

/// Subclasses the main window. Main thread (setup).
pub fn install(app: &AppHandle, window: &Window) {
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = HWND(hwnd.0);
    let created = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
    let taskbar =
        Box::new(Taskbar { app: app.clone(), created, list: None, added: false, icons: None, shows_pause: false });
    unsafe {
        // When the app runs elevated, Explorer's messages would otherwise be filtered out.
        for msg in [created, WM_COMMAND] {
            let _ = ChangeWindowMessageFilterEx(hwnd, msg, MSGFLT_ALLOW, None);
        }
        let taskbar = Box::into_raw(taskbar);
        if !SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, taskbar as usize).as_bool() {
            eprintln!("taskbar: SetWindowSubclass failed");
            drop(Box::from_raw(taskbar));
            return;
        }
        // Already on the taskbar: the message came before the subclass.
        if IsWindowVisible(hwnd).as_bool() {
            (*taskbar).button_created(hwnd);
        }
    }
    let rx = app.state::<Player>().subscribe();
    let (app, hwnd) = (app.clone(), hwnd.0 as isize);
    std::thread::spawn(move || worker(&app, hwnd, rx));
}

fn worker(app: &AppHandle, hwnd: isize, mut rx: Receiver<Event>) {
    let post = || unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as _)), WM_UPDATE, WPARAM(0), LPARAM(0));
    };
    loop {
        let shown = unsafe { IsWindowVisible(HWND(hwnd as _)).as_bool() };
        let tick = app.state::<Player>().current().filter(|s| shown && !s.paused).map(|s| {
            Duration::from_secs_f64(s.track.duration / 100.0).clamp(Duration::from_secs(1), Duration::from_secs(5))
        });
        // The timeout is made inside the async block: its timer needs the runtime (see lastfm.rs).
        let received = match tick {
            Some(d) => tauri::async_runtime::block_on(async { tokio::time::timeout(d, rx.recv()).await }).ok(),
            None => Some(tauri::async_runtime::block_on(rx.recv())),
        };
        match received {
            Some(Err(RecvError::Closed)) => break,
            _ => post(),
        }
    }
}

impl Taskbar {
    /// The taskbar button (re)appeared: add the buttons to it.
    fn button_created(&mut self, hwnd: HWND) {
        if self.list.is_none() {
            let list = unsafe {
                CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                    .and_then(|l| l.HrInit().map(|()| l))
            };
            match list {
                Ok(l) => self.list = Some(l),
                Err(e) => {
                    eprintln!("taskbar unavailable: {e}");
                    return;
                }
            }
        }
        if self.icons.is_none() {
            match make_icons(hwnd) {
                Ok(icons) => self.icons = Some(icons),
                Err(e) => eprintln!("taskbar: icons not made: {e}"),
            }
        }
        let (Some(list), Some(icons)) = (&self.list, &self.icons) else { return };
        self.shows_pause = self.app.state::<Player>().current().is_some_and(|s| !s.paused);
        let middle = if self.shows_pause { (icons[2], "Pause") } else { (icons[1], "Play") };
        let buttons = [(icons[0], "Previous"), middle, (icons[3], "Next")];
        let buttons: Vec<THUMBBUTTON> =
            buttons.iter().enumerate().map(|(i, (icon, tip))| button(i, *icon, tip)).collect();
        self.added = unsafe { list.ThumbBarAddButtons(hwnd, &buttons) }.is_ok();
        self.update(hwnd);
    }

    /// The play/pause button and the progress, from the player state.
    fn update(&mut self, hwnd: HWND) {
        let (Some(list), true) = (&self.list, unsafe { IsWindowVisible(hwnd).as_bool() }) else { return };
        let state = self.app.state::<Player>().current();
        let playing = state.as_ref().is_some_and(|s| !s.paused);
        if self.added
            && playing != self.shows_pause
            && let Some(icons) = &self.icons
        {
            let b = if playing { button(1, icons[2], "Pause") } else { button(1, icons[1], "Play") };
            if unsafe { list.ThumbBarUpdateButtons(hwnd, &[b]) }.is_ok() {
                self.shows_pause = playing;
            }
        }
        unsafe {
            match state.filter(|s| s.track.duration > 0.0) {
                Some(s) => {
                    let _ = list.SetProgressState(hwnd, if s.paused { TBPF_PAUSED } else { TBPF_NORMAL });
                    let total = (s.track.duration * 1000.0) as u64;
                    let _ = list.SetProgressValue(hwnd, ((s.position_now() * 1000.0) as u64).min(total), total);
                }
                None => {
                    let _ = list.SetProgressState(hwnd, TBPF_NOPROGRESS);
                }
            }
        }
    }
}

fn button(id: usize, icon: HICON, tip: &str) -> THUMBBUTTON {
    let mut b = THUMBBUTTON {
        dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
        iId: id as u32,
        hIcon: icon,
        dwFlags: THBF_ENABLED,
        ..Default::default()
    };
    for (dst, src) in b.szTip.iter_mut().zip(tip.encode_utf16().take(259)) {
        *dst = src;
    }
    b
}

/// The four glyphs as white icons with alpha, at the small icon size of the window's DPI.
fn make_icons(hwnd: HWND) -> Result<[HICON; 4]> {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }.max(16);
    let scale = size as f32 / 16.0;
    let format = gfx::text_format(ICON_FONT, 14.0, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_ALIGNMENT_CENTER)?;
    let mut icons = [HICON::default(); 4];
    for (icon, glyph) in icons.iter_mut().zip([PREVIOUS, PLAY, PAUSE, NEXT]) {
        let mut bitmap = gfx::Offscreen::new(size, size)?;
        bitmap.draw(scale, |c| c.text(glyph, &format, rect(0.0, 0.0, 16.0, 16.0), rgb(0xffffff)))?;
        // Icons take straight alpha, the bitmap has it premultiplied.
        for px in bitmap.pixels().as_chunks_mut::<4>().0 {
            let a = u32::from(px[3]);
            for c in &mut px[..3] {
                if let Some(v) = (u32::from(*c) * 255).checked_div(a) {
                    *c = v.min(255) as u8;
                }
            }
        }
        // The mask is unused with an alpha channel, but must be there.
        let zeros = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
        unsafe {
            let mask = CreateBitmap(size, size, 1, 1, Some(zeros.as_ptr().cast()));
            let info = ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: bitmap.bitmap(), ..Default::default() };
            let made = CreateIconIndirect(&info);
            let _ = DeleteObject(mask.into());
            *icon = made?;
        }
    }
    Ok(icons)
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let taskbar = unsafe { &mut *(data as *mut Taskbar) };
    match msg {
        _ if msg == taskbar.created => {
            taskbar.added = false;
            taskbar.button_created(hwnd);
        }
        WM_UPDATE => {
            taskbar.update(hwnd);
            return LRESULT(0);
        }
        WM_COMMAND if (wparam.0 >> 16) as u32 & 0xffff == THBN_CLICKED => {
            if let Some(command) = COMMANDS.get(wparam.0 & 0xffff) {
                crate::player(&taskbar.app, command);
            }
            return LRESULT(0);
        }
        WM_NCDESTROY => unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
            let taskbar = Box::from_raw(data as *mut Taskbar);
            for icon in taskbar.icons.into_iter().flatten() {
                let _ = DestroyIcon(icon);
            }
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
