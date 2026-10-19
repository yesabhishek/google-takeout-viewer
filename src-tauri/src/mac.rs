//! macOS fast paths: ImageIO for every still format the OS understands
//! (JPEG, HEIC, RAW, …) and AVFoundation for video frames. Both use hardware
//! decoders where available, which is much faster than decoding in Rust.

use objc2::rc::autoreleasepool;
use objc2_av_foundation::{AVAssetImageGenerator, AVURLAsset};
use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFMutableData, CFNumber, CFRetained, CFString, CFType, CFURL, CGSize,
};
use objc2_core_graphics::CGImage;
use objc2_core_media::CMTime;
use objc2_foundation::{NSString, NSURL};
use objc2_image_io::{
    kCGImageDestinationLossyCompressionQuality, kCGImagePropertyOrientation,
    kCGImagePropertyPixelHeight, kCGImagePropertyPixelWidth,
    kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCache, kCGImageSourceThumbnailMaxPixelSize, CGImageDestination,
    CGImageSource,
};
use std::path::Path;

type Dict = CFDictionary<CFString, CFType>;

fn dict(keys: &[&CFString], values: &[&CFType]) -> CFRetained<Dict> {
    Dict::from_slices(keys, values)
}

fn open_source(path: &Path) -> Option<CFRetained<CGImageSource>> {
    let url = CFURL::from_file_path(path)?;
    unsafe {
        let opts = dict(&[kCGImageSourceShouldCache], &[CFBoolean::new(false)]);
        CGImageSource::with_url(&url, Some(opts.as_opaque()))
    }
}

fn num(d: &CFDictionary, key: &CFString) -> Option<i64> {
    let d: &Dict = unsafe { &*(d as *const CFDictionary as *const Dict) };
    let v = d.get(key)?;
    v.downcast_ref::<CFNumber>()?.as_i64()
}

/// (width, height, EXIF orientation) without decoding pixels.
pub fn image_props(path: &Path) -> Option<(u32, u32, u32)> {
    autoreleasepool(|_| unsafe {
        let src = open_source(path)?;
        let idx = src.primary_image_index();
        let props = src.properties_at_index(idx, None)?;
        let w = num(&props, kCGImagePropertyPixelWidth)?;
        let h = num(&props, kCGImagePropertyPixelHeight)?;
        let o = num(&props, kCGImagePropertyOrientation).unwrap_or(1);
        Some((w as u32, h as u32, o as u32))
    })
}

/// Decode an image scaled so its long edge is at most `max_px`, with EXIF
/// orientation applied.
fn scaled_image(src: &CGImageSource, max_px: u32) -> Option<CFRetained<CGImage>> {
    unsafe {
        let max = CFNumber::new_i32(max_px as i32);
        let opts = dict(
            &[
                kCGImageSourceCreateThumbnailFromImageAlways,
                kCGImageSourceCreateThumbnailWithTransform,
                kCGImageSourceThumbnailMaxPixelSize,
                kCGImageSourceShouldCache,
            ],
            &[CFBoolean::new(true), CFBoolean::new(true), &max, CFBoolean::new(false)],
        );
        src.thumbnail_at_index(src.primary_image_index(), Some(opts.as_opaque()))
    }
}

fn encode(img: &CGImage, uti: &str, quality: f64) -> Option<Vec<u8>> {
    unsafe {
        let data = CFMutableData::new(None, 0)?;
        let ty = CFString::from_str(uti);
        let dest = CGImageDestination::with_data(&data, &ty, 1, None)?;
        let q = CFNumber::new_f64(quality);
        let props = dict(&[kCGImageDestinationLossyCompressionQuality], &[&q]);
        dest.add_image(img, Some(props.as_opaque()));
        if !dest.finalize() {
            return None;
        }
        Some(data.to_vec())
    }
}

pub fn image_thumbnail(path: &Path, max_px: u32, quality: f64) -> Option<Vec<u8>> {
    autoreleasepool(|_| {
        let src = open_source(path)?;
        let img = scaled_image(&src, max_px)?;
        encode(&img, "public.jpeg", quality)
    })
}

pub fn video_thumbnail(path: &Path, max_px: u32, quality: f64) -> Option<Vec<u8>> {
    autoreleasepool(|_| unsafe {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
        let generator = AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset);
        generator.setAppliesPreferredTrackTransform(true);
        generator.setMaximumSize(CGSize { width: max_px as f64, height: max_px as f64 });
        let tol = CMTime::with_seconds(2.0, 600);
        generator.setRequestedTimeToleranceBefore(tol);
        generator.setRequestedTimeToleranceAfter(tol);
        #[allow(deprecated)]
        let img = generator
            .copyCGImageAtTime_actualTime_error(CMTime::with_seconds(0.5, 600), std::ptr::null_mut())
            .or_else(|_| {
                generator.copyCGImageAtTime_actualTime_error(
                    CMTime::with_seconds(0.0, 600),
                    std::ptr::null_mut(),
                )
            })
            .ok()?;
        encode(&img, "public.jpeg", quality)
    })
}

/// UTIs ImageIO can write on this machine.
pub fn writable_types() -> Vec<String> {
    autoreleasepool(|_| unsafe {
        let arr = CGImageDestination::type_identifiers();
        let arr: &objc2_core_foundation::CFArray<CFString> =
            &*(&*arr as *const objc2_core_foundation::CFArray as *const objc2_core_foundation::CFArray<CFString>);
        arr.iter().map(|s| s.to_string()).collect()
    })
}

/// Convert `src_path` to `uti`, optionally scaling the long edge to `max_px`.
/// Full-size conversions keep all metadata (EXIF, GPS, orientation).
pub fn convert(src_path: &Path, dest_path: &Path, uti: &str, quality: f64, max_px: Option<u32>) -> Result<(), String> {
    autoreleasepool(|_| unsafe {
        let src = open_source(src_path).ok_or("could not read image")?;
        let url = CFURL::from_file_path(dest_path).ok_or("bad destination path")?;
        let ty = CFString::from_str(uti);
        let dest = CGImageDestination::with_url(&url, &ty, 1, None).ok_or("format not supported")?;
        let q = CFNumber::new_f64(quality);
        let idx = src.primary_image_index();
        match max_px {
            None => {
                let props = dict(&[kCGImageDestinationLossyCompressionQuality], &[&q]);
                dest.add_image_from_source(&src, idx, Some(props.as_opaque()));
            }
            Some(px) => {
                let img = scaled_image(&src, px).ok_or("could not decode image")?;
                // Orientation is baked into the pixels now.
                let one = CFNumber::new_i32(1);
                let props = dict(
                    &[kCGImageDestinationLossyCompressionQuality, kCGImagePropertyOrientation],
                    &[&q, &one],
                );
                dest.add_image(&img, Some(props.as_opaque()));
            }
        }
        if dest.finalize() {
            Ok(())
        } else {
            Err("could not write image".into())
        }
    })
}
