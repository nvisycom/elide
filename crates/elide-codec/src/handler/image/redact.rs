//! Shared image redaction: apply an [`ImageReplacement`] to a region of
//! a decoded [`DynamicImage`].
//!
//! Format-agnostic, so PNG, JPEG, and TIFF handlers all redact through
//! the same path. Each treatment crops the target region, transforms it,
//! and overlays the result back onto the image in place.

use elide_core::modality::image::ImageReplacement;
use elide_core::primitive::{BoundingBox, Color, Dimensions, PixelRegion};
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

/// Apply `replacement` to the `bounding_box` region of `img` in place.
///
/// Out-of-bounds or zero-area regions are skipped silently; redaction is
/// best-effort over whatever pixels actually exist.
pub(crate) fn apply(img: &mut DynamicImage, replacement: &ImageReplacement, bbox: &BoundingBox) {
    let (w, h) = img.dimensions();
    let Some(region) = bbox.to_pixels(Dimensions::new(w, h)) else {
        return;
    };
    match replacement {
        ImageReplacement::Blur { sigma } => blur(img, region, *sigma),
        ImageReplacement::Pixelate { block_size } => pixelate(img, region, *block_size),
        ImageReplacement::Block { color } => block(img, region, *color),
        ImageReplacement::Removed => block(img, region, Color::BLACK),
    }
}

/// Gaussian blur over the region: crop, blur the crop, overlay it back.
fn blur(img: &mut DynamicImage, region: PixelRegion, sigma: f32) {
    let sub = img
        .crop_imm(region.x, region.y, region.width, region.height)
        .to_rgba8();
    let blurred = imageproc::filter::gaussian_blur_f32(&sub, sigma.max(f32::MIN_POSITIVE));
    let (x, y) = (region.x as i64, region.y as i64);
    image::imageops::overlay(img, &DynamicImage::ImageRgba8(blurred), x, y);
}

/// Solid-color block over the region.
fn block(img: &mut DynamicImage, region: PixelRegion, color: Color) {
    let fill = RgbaImage::from_pixel(
        region.width,
        region.height,
        Rgba([color.r, color.g, color.b, 255]),
    );
    let (x, y) = (region.x as i64, region.y as i64);
    image::imageops::overlay(img, &DynamicImage::ImageRgba8(fill), x, y);
}

/// Mosaic pixelation: downscale the region with nearest-neighbor, scale
/// it back up, and overlay.
fn pixelate(img: &mut DynamicImage, region: PixelRegion, block_size: u32) {
    let block_size = block_size.max(1);
    let small_w = (region.width / block_size).max(1);
    let small_h = (region.height / block_size).max(1);
    let sub = img.crop_imm(region.x, region.y, region.width, region.height);
    let small = sub.resize_exact(small_w, small_h, FilterType::Nearest);
    let mosaic = small.resize_exact(region.width, region.height, FilterType::Nearest);
    let (x, y) = (region.x as i64, region.y as i64);
    image::imageops::overlay(img, &mosaic, x, y);
}
