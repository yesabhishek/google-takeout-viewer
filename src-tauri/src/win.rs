//! Windows fast paths: WIC (Windows Imaging Component) for still images and
//! Media Foundation for video frames. WIC reads JPEG, PNG, GIF, TIFF, BMP and
//! WebP out of the box, and HEIC/HEIF, AVIF and camera RAW once Microsoft's
//! free HEIF / AV1 / Raw Image Extensions are installed. Callers fall back to
//! the `image` crate and ffmpeg when these return `None`.

use image::{DynamicImage, RgbImage, RgbaImage};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::OnceLock;
use windows::core::{w, Interface, GUID, PCWSTR};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_MetadataFormatHeif, GUID_WICPixelFormat32bppRGBA, IWICBitmapFrameDecode,
    IWICBitmapSource, IWICImagingFactory, IWICMetadataBlockReader, IWICPalette, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand, WICHeifOrientation,
};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFSample, IMFSourceReader, MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromURL,
    MFMediaType_Video, MFStartup, MFVideoFormat_RGB32, MFSTARTUP_LITE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE,
    MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::{VT_I8, VT_UI2};

thread_local! {
    // One factory per thread. Threads that already joined a single-threaded
    // apartment (the UI thread) get RPC_E_CHANGED_MODE here, which is fine:
    // COM is usable there either way.
    static WIC: Option<IWICImagingFactory> = unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()
    };
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn with_wic<T>(f: impl FnOnce(&IWICImagingFactory) -> Option<T>) -> Option<T> {
    WIC.with(|w| w.as_ref().and_then(f))
}

fn open_frame(factory: &IWICImagingFactory, path: &Path) -> Option<IWICBitmapFrameDecode> {
    let name = wide(path);
    unsafe {
        let dec = factory
            .CreateDecoderFromFilename(PCWSTR(name.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .ok()?;
        dec.GetFrame(0).ok()
    }
}

fn ui2(v: &mut PROPVARIANT) -> Option<u32> {
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        let out = (inner.vt == VT_UI2).then(|| inner.Anonymous.uiVal as u32);
        let _ = PropVariantClear(v);
        out
    }
}

/// EXIF-style orientation (1–8) of a decoded frame, if the file records one.
/// The HEIF decoder doesn't rotate pixels itself; it reports the `irot`/`imir`
/// transform through its own metadata block, so that is checked first.
fn orientation(frame: &IWICBitmapFrameDecode) -> Option<u32> {
    unsafe {
        if let Ok(blocks) = frame.cast::<IWICMetadataBlockReader>() {
            for i in 0..blocks.GetCount().unwrap_or(0) {
                let Ok(reader) = blocks.GetReaderByIndex(i) else { continue };
                if reader.GetMetadataFormat().ok() != Some(GUID_MetadataFormatHeif) {
                    continue;
                }
                let mut id = PROPVARIANT::default();
                (*id.Anonymous.Anonymous).vt = VT_UI2;
                (*id.Anonymous.Anonymous).Anonymous.uiVal = WICHeifOrientation.0 as u16;
                let mut v = PROPVARIANT::default();
                if reader.GetValue(std::ptr::null(), &id, &mut v).is_ok() {
                    if let Some(o) = ui2(&mut v).filter(|o| (1..=8).contains(o)) {
                        return Some(o);
                    }
                }
            }
        }
        let query = frame.GetMetadataQueryReader().ok()?;
        for name in [w!("System.Photo.Orientation"), w!("/app1/ifd/{ushort=274}"), w!("/ifd/{ushort=274}")] {
            let mut v = PROPVARIANT::default();
            if query.GetMetadataByName(name, &mut v).is_ok() {
                if let Some(o) = ui2(&mut v).filter(|o| (1..=8).contains(o)) {
                    return Some(o);
                }
            }
        }
        None
    }
}

/// (width, height, EXIF orientation) without decoding pixels. Width and height
/// are as stored, before orientation is applied.
pub fn image_props(path: &Path) -> Option<(u32, u32, Option<u32>)> {
    with_wic(|f| unsafe {
        let frame = open_frame(f, path)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        (w > 0 && h > 0).then(|| (w, h, orientation(&frame)))
    })
}

/// Decode an image, scaled so its long edge is at most `max_px` when given,
/// with EXIF orientation applied. Scaling happens inside the codec where it
/// can (JPEG DCT scaling), so thumbnails of huge photos stay cheap.
pub fn decode(path: &Path, max_px: Option<u32>) -> Option<DynamicImage> {
    let (img, o) = with_wic(|f| unsafe {
        let frame = open_frame(f, path)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        if w == 0 || h == 0 {
            return None;
        }
        let o = orientation(&frame);
        let mut src: IWICBitmapSource = frame.cast().ok()?;
        if let Some(px) = max_px.filter(|&px| w.max(h) > px) {
            let s = px as f64 / w.max(h) as f64;
            let nw = ((w as f64 * s).round() as u32).max(1);
            let nh = ((h as f64 * s).round() as u32).max(1);
            let scaler = f.CreateBitmapScaler().ok()?;
            scaler.Initialize(&src, nw, nh, WICBitmapInterpolationModeFant).ok()?;
            src = scaler.cast().ok()?;
        }
        let conv = f.CreateFormatConverter().ok()?;
        conv.Initialize(
            &src,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None::<&IWICPalette>,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .ok()?;
        conv.GetSize(&mut w, &mut h).ok()?;
        let stride = w.checked_mul(4)?;
        let mut buf = vec![0u8; stride as usize * h as usize];
        conv.CopyPixels(std::ptr::null(), stride, &mut buf).ok()?;
        Some((RgbaImage::from_raw(w, h, buf)?, o))
    })?;
    let mut img = DynamicImage::ImageRgba8(img);
    if let Some(o) = o.and_then(|o| image::metadata::Orientation::from_exif(o as u8)) {
        img.apply_orientation(o);
    }
    Some(img)
}

pub fn image_thumbnail(path: &Path, max_px: u32, quality: u8) -> Option<Vec<u8>> {
    crate::thumbs::portable::encode_jpeg(&decode(path, Some(max_px))?, quality)
}

fn media_foundation() -> bool {
    static STARTED: OnceLock<bool> = OnceLock::new();
    *STARTED.get_or_init(|| unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE).is_ok() })
}

/// Read the next decoded frame, skipping stream ticks and gaps.
unsafe fn next_sample(reader: &IMFSourceReader, stream: u32) -> Option<IMFSample> {
    for _ in 0..64 {
        let mut flags = 0u32;
        let mut sample: Option<IMFSample> = None;
        reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample)).ok()?;
        if sample.is_some() {
            return sample;
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return None;
        }
    }
    None
}

/// Seek to `hns` (100 ns units) and decode one frame as RGB.
unsafe fn frame_at(reader: &IMFSourceReader, stream: u32, hns: i64) -> Option<RgbImage> {
    let mut pos = PROPVARIANT::default();
    (*pos.Anonymous.Anonymous).vt = VT_I8;
    (*pos.Anonymous.Anonymous).Anonymous.hVal = hns;
    reader.SetCurrentPosition(&GUID::zeroed(), &pos).ok()?;
    let sample = next_sample(reader, stream)?;

    // The output type is final once a sample has been delivered.
    let ty = reader.GetCurrentMediaType(stream).ok()?;
    let size = ty.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
    let (w, h) = ((size >> 32) as u32, size as u32);
    if w == 0 || h == 0 {
        return None;
    }
    // A negative stride means the frame is stored bottom-up.
    let stride = ty.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(w as i32 * 4);
    let pitch = stride.unsigned_abs() as usize;
    if pitch < w as usize * 4 {
        return None;
    }

    let buf = sample.ConvertToContiguousBuffer().ok()?;
    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    buf.Lock(&mut ptr, None, Some(&mut len)).ok()?;
    let img = if ptr.is_null() || (len as usize) < pitch * h as usize {
        None
    } else {
        let data = std::slice::from_raw_parts(ptr, len as usize);
        let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
        for y in 0..h as usize {
            let row = if stride < 0 { h as usize - 1 - y } else { y };
            for px in data[row * pitch..row * pitch + w as usize * 4].chunks_exact(4) {
                rgb.extend_from_slice(&[px[2], px[1], px[0]]); // BGRX -> RGB
            }
        }
        RgbImage::from_raw(w, h, rgb)
    };
    let _ = buf.Unlock();
    img
}

unsafe fn video_frame(path: &Path) -> Option<RgbImage> {
    let mut attrs: Option<IMFAttributes> = None;
    MFCreateAttributes(&mut attrs, 1).ok()?;
    let attrs = attrs?;
    // Lets the reader convert whatever the decoder outputs (NV12, …) to RGB32.
    attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1).ok()?;
    let url = wide(path);
    let reader = MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), &attrs).ok()?;

    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).ok()?;
    reader.SetStreamSelection(stream, true).ok()?;
    let ty = MFCreateMediaType().ok()?;
    ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).ok()?;
    ty.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).ok()?;
    reader.SetCurrentMediaType(stream, None, &ty).ok()?;

    frame_at(&reader, stream, 5_000_000).or_else(|| frame_at(&reader, stream, 0))
}

/// Poster frame at ~0.5 s. Media Foundation decodes H.264 out of the box and
/// HEVC when the HEVC Video Extensions are installed; anything else returns
/// `None` so the caller can try ffmpeg.
pub fn video_thumbnail(path: &Path, max_px: u32, quality: u8) -> Option<Vec<u8>> {
    if !media_foundation() || !WIC.with(|w| w.is_some()) {
        return None; // the WIC thread-local also initialises COM for this thread
    }
    let frame = unsafe { video_frame(path) }?;
    // Media Foundation hands back frames as stored; apply the track rotation.
    let rotation = crate::video::probe(path).map(|v| v.rotation).unwrap_or(0);
    let img = DynamicImage::ImageRgb8(frame);
    let img = match rotation {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img,
    };
    let img = if img.width().max(img.height()) > max_px { img.thumbnail(max_px, max_px) } else { img };
    crate::thumbs::portable::encode_jpeg(&img, quality)
}
