//! Cover art, shared by the mini player, the album color, share lyrics and ambient mode: the validated
//! artwork URL is downloaded (https on one of the site's image hosts, no redirects, at most 4 MB) and
//! decoded with WIC into a small premultiplied BGRA buffer. The last cover is kept, so two consumers of
//! the same track download it once. Also a soft color field of a cover (`field`) and PNG encoding.
//! Call these from a worker thread, never from the UI thread.

use std::cell::Cell;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, GUID_WICPixelFormat32bppPBGRA,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache, WICBitmapInterpolationModeFant,
    WICBitmapInterpolationModeHighQualityCubic, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand, WICRect,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STATFLAG_NONAME, STATSTG,
    STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::{Error, Result};

/// Decoded artwork side in pixels: the 80 DIP square of the mini player at up to 200% scale.
const ART_PIXELS: u32 = 160;
const MAX_ART_BYTES: u64 = 4 * 1024 * 1024;

pub struct Art {
    pub url: String,
    pub width: u32,
    pub height: u32,
    /// Premultiplied BGRA, `width * 4` bytes per row.
    pub pixels: Vec<u8>,
}

static LAST: Mutex<Option<Arc<Art>>> = Mutex::new(None);

/// The decoded cover of `url`. Concurrent callers for different covers wait for each other.
pub fn load(url: &str) -> std::result::Result<Arc<Art>, String> {
    // Same check as the page payload: https on one of the site's image hosts, nothing else.
    let url = crate::player::artwork(url).ok_or("not an artwork URL")?;
    let mut last = LAST.lock().unwrap();
    if let Some(art) = last.as_ref().filter(|a| a.url == url) {
        return Ok(art.clone());
    }
    let mut response = agent().get(&url).call().map_err(|e| e.to_string())?;
    if response.status() != 200 {
        return Err(format!("HTTP {}", response.status()));
    }
    let bytes = response.body_mut().with_config().limit(MAX_ART_BYTES).read_to_vec().map_err(|e| e.to_string())?;
    let (width, height, pixels) = decode(&bytes).map_err(|e| e.to_string())?;
    let art = Arc::new(Art { url, width, height, pixels });
    *last = Some(art.clone());
    Ok(art)
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .tls_config(
                    TlsConfig::builder()
                        .provider(TlsProvider::NativeTls)
                        .root_certs(RootCerts::PlatformVerifier)
                        .build(),
                )
                .user_agent(concat!("feathertune/", env!("CARGO_PKG_VERSION")))
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(15)))
                .build(),
        )
    })
}

/// Any format WIC knows, scaled to fit `ART_PIXELS` and converted to premultiplied BGRA.
fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    thread_local! {
        static COM: Cell<bool> = const { Cell::new(false) };
    }
    if !COM.replace(true) {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    }
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h)?;
        if w == 0 || h == 0 {
            return Err(Error::from_hresult(E_FAIL));
        }
        let fit = (ART_PIXELS as f32 / w.min(h) as f32).min(1.0);
        let (w, h) = (((w as f32 * fit).round() as u32).max(1), ((h as f32 * fit).round() as u32).max(1));
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&frame, w, h, WICBitmapInterpolationModeFant)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        converter.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
        Ok((w, h, pixels))
    }
}

/// The cover's center square shrunk to `cells` × `cells` and blown up to `side` × `side` with a cubic
/// filter: a soft field of its colors, already as blurred as it can get, so it is shown large with no
/// blur. Its colors move away from gray by `saturation` and its light is scaled by `brightness`.
/// Premultiplied BGRA.
pub fn field(art: &Art, cells: u32, side: u32, saturation: f32, brightness: f32) -> Result<Vec<u8>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let source = wic.CreateBitmapFromMemory(
            art.width,
            art.height,
            &GUID_WICPixelFormat32bppPBGRA,
            art.width * 4,
            &art.pixels,
        )?;
        let square = art.width.min(art.height);
        let square = WICRect {
            X: ((art.width - square) / 2) as i32,
            Y: ((art.height - square) / 2) as i32,
            Width: square as i32,
            Height: square as i32,
        };
        let clipper = wic.CreateBitmapClipper()?;
        clipper.Initialize(&source, &square)?;
        let small = wic.CreateBitmapScaler()?;
        small.Initialize(&clipper, cells, cells, WICBitmapInterpolationModeFant)?;
        let big = wic.CreateBitmapScaler()?;
        big.Initialize(&small, side, side, WICBitmapInterpolationModeHighQualityCubic)?;
        let mut pixels = vec![0u8; (side * side * 4) as usize];
        big.CopyPixels(std::ptr::null(), side * 4, &mut pixels)?;
        for px in pixels.as_chunks_mut::<4>().0 {
            let gray = (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) as f32 / 3.0;
            let alpha = f32::from(px[3]);
            for c in &mut px[..3] {
                *c = ((gray + (f32::from(*c) - gray) * saturation) * brightness).clamp(0.0, alpha) as u8;
            }
        }
        Ok(pixels)
    }
}

/// Encodes opaque top-down BGRA pixels as PNG.
pub fn png(pixels: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = SHCreateMemStream(None).ok_or_else(|| Error::from_hresult(E_FAIL))?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let (mut frame, mut options) = (None, None);
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.ok_or_else(|| Error::from_hresult(E_FAIL))?;
        frame.Initialize(options.as_ref())?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        if format != GUID_WICPixelFormat32bppBGRA {
            return Err(Error::from_hresult(E_FAIL));
        }
        frame.WritePixels(h, w * 4, pixels)?;
        frame.Commit()?;
        encoder.Commit()?;
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut bytes = vec![0u8; stat.cbSize as usize];
        let mut read = 0u32;
        stream.Read(bytes.as_mut_ptr().cast(), bytes.len() as u32, Some(&mut read)).ok()?;
        bytes.truncate(read as usize);
        Ok(bytes)
    }
}
