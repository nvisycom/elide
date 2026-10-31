//! Decode an image XObject stored as raw samples (Flate, LZW, or unfiltered)
//! to pixels, so it can be surfaced to the image pipeline like a JPEG.
//!
//! Every lossless image a PDF generator embeds (a PNG, a screenshot, a logo)
//! is stored this way, as are the rasters this crate writes when it flattens a
//! page. The supported samples are 1, 2, 4, or 8 bits per component in a gray,
//! RGB, or CMYK space (device, calibrated, or ICC-based by its component
//! count), or indexed into a gray or RGB palette. Anything else (a stencil
//! mask, an unusual colour space, a non-default decode array other than an
//! inverted gray) is not surfaceable, and the sanitizer strips it.
//!
//! Whether an image is supported is read from its dictionary alone, so the
//! sanitizer's check costs no decompression; the samples are decoded once, when
//! the image is surfaced.

use std::io::Cursor;

use elide_core::{Error, ErrorKind, Result};
use image::{DynamicImage, GrayImage, ImageFormat, RgbImage};
use lopdf::{Dictionary, Document, Object, Stream};

/// The largest decoded image accepted, in bytes: a sample stream may claim any
/// size, so its decode is bounded to refuse a decompression bomb.
const MAX_SAMPLE_BYTES: usize = 256 * 1024 * 1024;

/// An image's decoded pixels: 8-bit gray or RGB, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pixels {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Whether each pixel is three RGB bytes (else one gray byte).
    pub(crate) rgb: bool,
    pub(crate) data: Vec<u8>,
}

/// How a sample's components map to a pixel.
enum Space {
    Gray {
        inverted: bool,
    },
    Rgb,
    Cmyk,
    /// A palette of `base` colours, `palette` bytes long.
    Indexed {
        rgb: bool,
        palette: Vec<u8>,
    },
}

impl Space {
    fn components(&self) -> usize {
        match self {
            Self::Gray { .. } | Self::Indexed { .. } => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }
}

/// The shape of an image's samples, read from its dictionary.
struct Layout {
    width: u32,
    height: u32,
    bits: usize,
    space: Space,
}

impl Layout {
    /// The bytes one row of samples takes, padded to a whole byte.
    fn row_bytes(&self) -> usize {
        (self.width as usize * self.space.components() * self.bits).div_ceil(8)
    }
}

/// Whether the raw-sample image XObject `dict` in `doc` describes samples this
/// module decodes.
pub(crate) fn is_supported(doc: &Document, dict: &Dictionary) -> bool {
    layout(doc, dict).is_some()
}

/// The layout `dict` describes, or `None` when its samples are of a kind this
/// module does not decode or claim more than [`MAX_SAMPLE_BYTES`].
fn layout(doc: &Document, dict: &Dictionary) -> Option<Layout> {
    let mask = dict
        .get(b"ImageMask")
        .and_then(Object::as_bool)
        .unwrap_or(false);
    if mask {
        return None;
    }
    let width = u32::try_from(dict.get(b"Width").ok()?.as_i64().ok()?).ok()?;
    let height = u32::try_from(dict.get(b"Height").ok()?.as_i64().ok()?).ok()?;
    let bits = dict.get(b"BitsPerComponent").ok()?.as_i64().ok()?;
    if ![1, 2, 4, 8].contains(&bits) || width == 0 || height == 0 {
        return None;
    }
    let layout = Layout {
        width,
        height,
        bits: bits as usize,
        space: space(
            doc,
            resolve(doc, dict.get(b"ColorSpace").ok()?)?,
            decode_array(dict),
        )?,
    };
    let size = layout.row_bytes().checked_mul(height as usize)?;
    (size <= MAX_SAMPLE_BYTES).then_some(layout)
}

/// The pixels of the raw-sample image XObject `stream` in `doc`. Samples
/// shorter than the image are padded with zeros, as a viewer draws them.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the image is not of a supported kind (see
/// [`is_supported`]), its samples cannot be decompressed, or an index falls
/// outside its palette.
pub(crate) fn decode(doc: &Document, stream: &Stream) -> Result<Pixels> {
    let malformed = |reason: &str| Error::new(ErrorKind::MalformedInput, reason.to_owned());
    let layout = layout(doc, &stream.dict).ok_or_else(|| malformed("unsupported image samples"))?;
    let row_bytes = layout.row_bytes();
    let size = row_bytes * layout.height as usize;
    let mut samples = stream
        .decompressed_content_with_limit(MAX_SAMPLE_BYTES)
        .map_err(|e| malformed(&format!("image samples: {e}")))?;
    samples.resize(size, 0);

    let count = layout.width as usize * layout.space.components();
    let mut data = Vec::with_capacity(layout.width as usize * layout.height as usize * 3);
    for row in samples.chunks_exact(row_bytes) {
        let values = unpack(row, layout.bits, count);
        pixels(&layout.space, layout.bits, &values, &mut data)
            .ok_or_else(|| malformed("image palette index out of range"))?;
    }
    let rgb = !matches!(
        layout.space,
        Space::Gray { .. } | Space::Indexed { rgb: false, .. }
    );
    Ok(Pixels {
        width: layout.width,
        height: layout.height,
        rgb,
        data,
    })
}

/// The pixels of `stream` encoded as a PNG, the form the image pipeline reads.
///
/// # Errors
///
/// As [`decode`].
pub(crate) fn png(doc: &Document, stream: &Stream) -> Result<Vec<u8>> {
    let Pixels {
        width,
        height,
        rgb,
        data,
    } = decode(doc, stream)?;
    let image = if rgb {
        RgbImage::from_raw(width, height, data).map(DynamicImage::ImageRgb8)
    } else {
        GrayImage::from_raw(width, height, data).map(DynamicImage::ImageLuma8)
    };
    let image = image.ok_or_else(|| {
        Error::new(
            ErrorKind::MalformedInput,
            "image pixels do not fill the image",
        )
    })?;
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("encode PNG: {e}")))?;
    Ok(out)
}

/// `object`, following a reference.
fn resolve<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

/// Whether the image's `/Decode` array inverts a gray sample, or `None` when
/// it maps samples some other non-default way.
fn decode_array(dict: &Dictionary) -> Option<bool> {
    let Ok(array) = dict.get(b"Decode").and_then(Object::as_array) else {
        return Some(false);
    };
    let values: Vec<f32> = array.iter().filter_map(|v| v.as_float().ok()).collect();
    let default = values.chunks(2).all(|pair| pair == [0.0, 1.0]);
    let inverted = values == [1.0, 0.0];
    if default {
        Some(false)
    } else if inverted {
        Some(true)
    } else {
        None
    }
}

/// The colour space `object` names, with `inverted` from the decode array
/// (`None` when the array is unsupported).
fn space(doc: &Document, object: &Object, inverted: Option<bool>) -> Option<Space> {
    let inverted = inverted?;
    let by_components = |n: i64| match n {
        1 => Some(Space::Gray { inverted }),
        3 if !inverted => Some(Space::Rgb),
        4 if !inverted => Some(Space::Cmyk),
        _ => None,
    };
    match object {
        Object::Name(name) => match name.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => by_components(1),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => by_components(3),
            b"DeviceCMYK" | b"CMYK" => by_components(4),
            _ => None,
        },
        Object::Array(array) => {
            let family = array.first()?.as_name().ok()?;
            match family {
                b"CalGray" => by_components(1),
                b"CalRGB" => by_components(3),
                b"ICCBased" => {
                    let profile = resolve(doc, array.get(1)?)?.as_stream().ok()?;
                    by_components(profile.dict.get(b"N").ok()?.as_i64().ok()?)
                }
                b"Indexed" | b"I" if !inverted => {
                    let base = space(doc, resolve(doc, array.get(1)?)?, Some(false))?;
                    let rgb = match base {
                        Space::Gray { .. } => false,
                        Space::Rgb => true,
                        _ => return None,
                    };
                    let hival = usize::try_from(array.get(2)?.as_i64().ok()?).ok()?;
                    let palette = match resolve(doc, array.get(3)?)? {
                        Object::String(bytes, _) => bytes.clone(),
                        Object::Stream(stream) => stream.decompressed_content().ok()?,
                        _ => return None,
                    };
                    let needed = (hival + 1) * if rgb { 3 } else { 1 };
                    (palette.len() >= needed).then_some(Space::Indexed { rgb, palette })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The first `count` samples of a `bits`-per-sample `row`, each as its raw
/// integer value.
fn unpack(row: &[u8], bits: usize, count: usize) -> Vec<u8> {
    if bits == 8 {
        return row[..count].to_vec();
    }
    let per_byte = 8 / bits;
    let mask = (1u8 << bits) - 1;
    (0..count)
        .map(|i| {
            let byte = row[i / per_byte];
            let shift = 8 - bits * (i % per_byte + 1);
            (byte >> shift) & mask
        })
        .collect()
}

/// Append the pixels of one row of sample `values` in `space` to `out`, or
/// `None` when an index falls outside its palette.
fn pixels(space: &Space, bits: usize, values: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let max = (1u16 << bits) - 1;
    let scale = |v: u8| u8::try_from(u16::from(v) * 255 / max).unwrap_or(u8::MAX);
    match space {
        Space::Gray { inverted } => out.extend(values.iter().map(|&v| {
            let gray = scale(v);
            if *inverted { 255 - gray } else { gray }
        })),
        Space::Rgb => out.extend(values.iter().map(|&v| scale(v))),
        Space::Cmyk => {
            for cmyk in values.chunks_exact(4) {
                let k = u16::from(255 - scale(cmyk[3]));
                for &ink in &cmyk[..3] {
                    let value = u16::from(255 - scale(ink)) * k / 255;
                    out.push(u8::try_from(value).unwrap_or(u8::MAX));
                }
            }
        }
        Space::Indexed { rgb, palette } => {
            let width = if *rgb { 3 } else { 1 };
            for &index in values {
                let at = usize::from(index) * width;
                out.extend_from_slice(palette.get(at..at + width)?);
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use lopdf::{StringFormat, dictionary};

    use super::*;

    fn image(dict: Dictionary, samples: Vec<u8>) -> Stream {
        Stream::new(dict, samples)
    }

    #[test]
    fn decodes_gray_rgb_and_indexed_samples() {
        let doc = Document::new();
        let gray = image(
            dictionary! { "Width" => 2, "Height" => 1, "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray" },
            vec![0, 255],
        );
        assert_eq!(decode(&doc, &gray).unwrap().data, [0, 255]);

        let rgb = image(
            dictionary! { "Width" => 1, "Height" => 1, "BitsPerComponent" => 8, "ColorSpace" => "DeviceRGB" },
            vec![10, 20, 30],
        );
        let pixels = decode(&doc, &rgb).unwrap();
        assert!(pixels.rgb);
        assert_eq!(pixels.data, [10, 20, 30]);

        // Two 1-bit pixels per row, rows padded to a byte: 0b10 in the top bits.
        let bitonal = image(
            dictionary! { "Width" => 2, "Height" => 1, "BitsPerComponent" => 1, "ColorSpace" => "DeviceGray" },
            vec![0b1000_0000],
        );
        assert_eq!(decode(&doc, &bitonal).unwrap().data, [255, 0]);

        let indexed = image(
            dictionary! {
                "Width" => 2, "Height" => 1, "BitsPerComponent" => 8,
                "ColorSpace" => vec![
                    Object::Name(b"Indexed".to_vec()),
                    Object::Name(b"DeviceRGB".to_vec()),
                    1.into(),
                    Object::String(vec![255, 0, 0, 0, 0, 255], StringFormat::Hexadecimal),
                ],
            },
            vec![1, 0],
        );
        assert_eq!(decode(&doc, &indexed).unwrap().data, [0, 0, 255, 255, 0, 0]);
    }

    #[test]
    fn a_stencil_mask_is_not_supported() {
        let doc = Document::new();
        let mask = dictionary! { "Width" => 8, "Height" => 1, "ImageMask" => true, "BitsPerComponent" => 1 };
        assert!(!is_supported(&doc, &mask));
        assert!(decode(&doc, &image(mask, vec![0xFF])).is_err());
    }

    #[test]
    fn short_samples_are_padded() {
        let doc = Document::new();
        let short = image(
            dictionary! { "Width" => 2, "Height" => 2, "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray" },
            vec![9; 3],
        );
        assert_eq!(decode(&doc, &short).unwrap().data, [9, 9, 9, 0]);
    }
}
