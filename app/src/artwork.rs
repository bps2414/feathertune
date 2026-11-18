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
const MAX_SOURCE_SIDE: u32 = 8192;
const MAX_SCALED_SIDE: f32 = 1024.0;
const MAX_ART_BYTES: u64 = 4 * 1024 * 1024;

pub struct Art {
    pub url: String,
    pub width: u32,
    pub height: u32,
    /// Premultiplied BGRA, `width * 4` bytes per row.
    pub pixels: Vec<u8>,
}

static LAST: Mutex<Option<Arc<Art>>> = Mutex::new(None);

/// The decoded cover of `url`.
pub fn load(url: &str) -> std::result::Result<Arc<Art>, String> {
    // Same check as the page payload: https on one of the site's image hosts, nothing else.
    let url = crate::player::artwork(url).ok_or("not an artwork URL")?;
    if let Some(art) = LAST.lock().unwrap_or_else(|e| e.into_inner()).as_ref().filter(|a| a.url == url) {
        return Ok(art.clone());
    }
    let mut response = agent().get(&url).call().map_err(|e| e.to_string())?;
    if response.status() != 200 {
        return Err(format!("HTTP {}", response.status()));
    }
    let bytes = response.body_mut().with_config().limit(MAX_ART_BYTES).read_to_vec().map_err(|e| e.to_string())?;
    let (width, height, pixels) = decode(&bytes).map_err(|e| e.to_string())?;
    let art = Arc::new(Art { url, width, height, pixels });
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(art.clone());
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
        if w == 0 || h == 0 || w > MAX_SOURCE_SIDE || h > MAX_SOURCE_SIDE {
            return Err(Error::from_hresult(E_FAIL));
        }
        let fit = (ART_PIXELS as f32 / w.min(h) as f32).min(MAX_SCALED_SIDE / w.max(h) as f32).min(1.0);
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
        let len = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| Error::from_hresult(E_FAIL))?;
        let mut pixels = vec![0u8; len];
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

/// The whole cover shrunk to `side` × `side`, blurred (a Gaussian of `sigma` pixels that fades into
/// transparency at the edges, like a CSS `blur()` of a box), with `saturation` and `brightness` as the CSS
/// filters of those names, and its alpha fading from full at `solid` of the way from the center to the
/// middle of a side to nothing at the side: a glow of the cover to show large behind it, with no filter
/// or mask left for the page to run. BGRA with straight alpha (what `png` takes).
pub fn halo(art: &Art, side: u32, sigma: f32, saturation: f32, brightness: f32, solid: f32) -> Result<Vec<u8>> {
    let small = unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let source = wic.CreateBitmapFromMemory(
            art.width,
            art.height,
            &GUID_WICPixelFormat32bppPBGRA,
            art.width * 4,
            &art.pixels,
        )?;
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&source, side, side, WICBitmapInterpolationModeFant)?;
        let mut pixels = vec![0u8; (side * side * 4) as usize];
        scaler.CopyPixels(std::ptr::null(), side * 4, &mut pixels)?;
        pixels
    };
    Ok(halo_pixels(&small, side as usize, sigma, saturation, brightness, solid))
}

/// `halo` on a premultiplied BGRA square of `n` × `n` pixels.
fn halo_pixels(src: &[u8], n: usize, sigma: f32, saturation: f32, brightness: f32, solid: f32) -> Vec<u8> {
    let radius = (sigma * 3.0).ceil() as isize;
    let kernel: Vec<f32> = (-radius..=radius).map(|i| (-0.5 * (i as f32 / sigma).powi(2)).exp()).collect();
    let total: f32 = kernel.iter().sum();
    // One pass along `step`; what is outside the square is transparent, so the edges fade out.
    let blur = |from: &[f32], step: usize, lines: usize| {
        let mut to = vec![0.0f32; from.len()];
        for line in 0..n {
            for at in 0..n {
                let mut sum = [0.0f32; 4];
                for (k, w) in kernel.iter().enumerate() {
                    let i = at as isize + k as isize - radius;
                    if i < 0 || i >= n as isize {
                        continue;
                    }
                    let p = (line * lines + i as usize * step) * 4;
                    for c in 0..4 {
                        sum[c] += from[p + c] * w;
                    }
                }
                let p = (line * lines + at * step) * 4;
                for c in 0..4 {
                    to[p + c] = sum[c] / total;
                }
            }
        }
        to
    };
    let floats: Vec<f32> = src.iter().map(|&b| f32::from(b)).collect();
    let rows = blur(&floats, 1, n);
    let both = blur(&rows, n, 1);
    let mut out = vec![0u8; n * n * 4];
    let half = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let p = (y * n + x) * 4;
            let alpha = both[p + 3];
            if alpha <= 0.0 {
                continue;
            }
            // Straight color, then the two CSS filters: saturate (its luminance matrix) and brightness.
            let [b, g, r] = [0, 1, 2].map(|c| both[p + c] / alpha * 255.0);
            let luma = 0.213 * r + 0.715 * g + 0.072 * b;
            for (c, v) in [b, g, r].into_iter().enumerate() {
                out[p + c] = ((luma + (v - luma) * saturation) * brightness).round().clamp(0.0, 255.0) as u8;
            }
            let far = ((x as f32 + 0.5 - half).hypot(y as f32 + 0.5 - half) / half).min(1.0);
            let mask = if far <= solid { 1.0 } else { (1.0 - far) / (1.0 - solid) };
            out[p + 3] = (alpha * mask).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Encodes top-down BGRA pixels (straight alpha) as PNG.
pub fn png(pixels: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_halo_is_solid_in_the_middle_and_gone_at_the_sides() {
        // An opaque square of one color (premultiplied BGRA).
        let n = 32;
        let src: Vec<u8> = [40u8, 80, 200, 255].repeat(n * n);
        let out = halo_pixels(&src, n, 1.5, 1.0, 1.0, 0.45);
        let at = |x: usize, y: usize| &out[(y * n + x) * 4..(y * n + x) * 4 + 4];
        // The color is kept where there is any, and the middle is opaque.
        assert_eq!(at(n / 2, n / 2), [40, 80, 200, 255]);
        // Halfway out the alpha is on its way down; at a side and in a corner nothing is left.
        let mid = at(n / 2 + n * 3 / 8, n / 2)[3];
        assert!(mid > 0 && mid < 255, "{mid}");
        assert!(at(n - 1, n / 2)[3] < 16 && at(0, 0)[3] == 0);
        // Brightness scales the color only.
        let dark = halo_pixels(&src, n, 1.5, 1.0, 0.5, 0.45);
        assert_eq!(&dark[(n / 2 * n + n / 2) * 4..][..4], [20, 40, 100, 255]);
    }

    #[test]
    fn png_round_trips_and_decode_rejects_oversized_frames() {
        let src = [40u8, 80, 200, 255].repeat(4 * 4);
        let bytes = png(&src, 4, 4).unwrap();
        let (w, h, out) = decode(&bytes).unwrap();
        assert_eq!((w, h, out.len()), (4, 4, 64));

        // Craft a 10000x10000 PNG IHDR and confirm decode rejects before WIC allocates gigabytes.
        let mut big = bytes.clone();
        if big.len() > 24 {
            big[16..20].copy_from_slice(&10_000u32.to_be_bytes());
            big[20..24].copy_from_slice(&10_000u32.to_be_bytes());
            assert!(decode(&big).is_err());
        }
    }
}
