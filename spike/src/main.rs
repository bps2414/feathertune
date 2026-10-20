// Measurement spike: a bare WebView2 window opening a URL.
// Usage: spike <url> <data-dir> [hide-after-s] [extra-webview2-args]
// With hide-after-s, hides the window and requests MemoryUsageTargetLevel=Low (tray scenario).
// Every 10 s prints the <video> time to prove playback keeps going.

use std::time::{Duration, Instant};

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtWindows};

const DEFAULT_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";
const PROBE: &str = "(() => { const v = document.querySelector('video'); return v ? [Math.round(v.currentTime), v.paused, v.videoHeight] : null })()";

fn main() -> wry::Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args.next().unwrap_or_else(|| "https://music.youtube.com".into());
    let data = args.next().unwrap_or_else(|| "spike-data".into());
    let hide_after = args.next().and_then(|s| s.parse::<u64>().ok()).filter(|s| *s > 0);
    let extra = args.next().unwrap_or_default();

    let event_loop = EventLoopBuilder::new().build();
    let window = WindowBuilder::new()
        .with_title("feathertune spike")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0))
        .build(&event_loop)
        .expect("window");
    let mut ctx = WebContext::new(Some(data.into()));
    let webview = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url(url)
        .with_additional_browser_args(&format!("{DEFAULT_ARGS} {extra}"))
        .build(&window)?;

    let start = Instant::now();
    let mut next_probe = start + Duration::from_secs(10);
    let mut hidden = false;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(500));
        let now = Instant::now();
        if let Some(s) = hide_after {
            if !hidden && now.duration_since(start) >= Duration::from_secs(s) {
                window.set_visible(false);
                let _ = webview.set_visible(false);
                low_memory(&webview);
                hidden = true;
                println!("{:>4}s hidden + memory Low", s);
            }
        }
        if now >= next_probe {
            let t = now.duration_since(start).as_secs();
            let _ = webview.evaluate_script_with_callback(PROBE, move |r| println!("{t:>4}s video={r}"));
            next_probe = now + Duration::from_secs(10);
        }
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    });
}

fn low_memory(webview: &wry::WebView) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, ICoreWebView2_19,
    };
    use windows_core::Interface;
    use wry::WebViewExtWindows;
    unsafe {
        let core = webview.controller().CoreWebView2().expect("core");
        let w19: ICoreWebView2_19 = core.cast().expect("ICoreWebView2_19");
        w19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW).expect("low");
    }
}
