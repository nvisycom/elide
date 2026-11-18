//! Decode an image XObject stored as raw samples (Flate, LZW, or unfiltered)
//! to pixels, so it can be surfaced to the image pipeline like a JPEG.
//!
//! Every lossless image a PDF generator embeds (a PNG, a screenshot, a logo)
//! is stored this way, as are the rasters this crate writes when it flattens a
//! page. The supported samples are 1, 2, 4, or 8 bits per component in a gray,
//! RGB, or CMYK space (device, calibrated, or ICC-based by its component
//! count), or indexed into a gray or RGB palette, and stencil masks, read as
//! 1-bit gray with the painted samples black. Anything else (an unusual colour
//! space, a non-default decode array other than an inverted gray) is not
//! surfaceable, and the rebuild removes it.
//!
//! Whether an image is supported is read from its dictionary alone, so the
//! rebuild's check costs no decompression; the samples are decoded once, when
//! the image is surfaced.

use std::io::Cursor;

use ::image::{DynamicImage, GrayImage, ImageFormat, RgbImage};
use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Object, Stream};

use crate::pdf::reader::DocumentExt;

/// The largest image accepted, in bytes, both as packed samples and as the
/// pixels they decode to: a sample stream may claim any size, and a 1-bit
/// sample expands to a whole byte per channel, so both are bounded to refuse
/// a decompression bomb.
const MAX_SAMPLE_BYTES: usize = 256 * 1024 * 1024;

/// The most bytes a palette takes: 256 entries of three components.
const MAX_PALETTE_BYTES: usize = 256 * 3;

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

    /// Whether a pixel decodes to three RGB bytes, else one gray byte.
    fn rgb(&self) -> bool {
        !matches!(self, Self::Gray { .. } | Self::Indexed { rgb: false, .. })
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

    /// The bytes the decoded pixels take, or `None` past `usize`.
    fn pixel_bytes(&self) -> Option<usize> {
        let channels = if self.space.rgb() { 3 } else { 1 };
        (self.width as usize)
            .checked_mul(self.height as usize)?
            .checked_mul(channels)
    }
}

/// Whether the image XObject `dict` is a stencil mask: its samples say where
/// to paint, not what colour.
pub(crate) fn is_stencil(doc: &Document, dict: &Dictionary) -> bool {
    dict.get(b"ImageMask")
        .ok()
        .and_then(|mask| doc.resolve(mask))
        .and_then(|mask| mask.as_bool().ok())
        .unwrap_or(false)
}

/// Whether the raw-sample image XObject `dict` in `doc` describes samples this
/// module decodes.
pub(crate) fn is_supported(doc: &Document, dict: &Dictionary) -> bool {
    layout(doc, dict).is_some()
}

/// The layout `dict` describes, or `None` when its samples are of a kind this
/// module does not decode, or they or their pixels take more than
/// [`MAX_SAMPLE_BYTES`].
fn layout(doc: &Document, dict: &Dictionary) -> Option<Layout> {
    let integer = |key: &[u8]| doc.resolve(dict.get(key).ok()?)?.as_i64().ok();
    let width = u32::try_from(integer(b"Width")?).ok()?;
    let height = u32::try_from(integer(b"Height")?).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let layout = if is_stencil(doc, dict) {
        // A stencil mask's samples are 1 bit, a 0 painting under the default
        // decode array (8.9.6.2): as gray, the painted samples are black.
        let bits = integer(b"BitsPerComponent").unwrap_or(1);
        if bits != 1 {
            return None;
        }
        Layout {
            width,
            height,
            bits: 1,
            space: Space::Gray {
                inverted: decode_array(doc, dict)?,
            },
        }
    } else {
        let bits = integer(b"BitsPerComponent")?;
        if ![1, 2, 4, 8].contains(&bits) {
            return None;
        }
        Layout {
            width,
            height,
            bits: bits as usize,
            space: space(
                doc,
                doc.resolve(dict.get(b"ColorSpace").ok()?)?,
                decode_array(doc, dict),
            )?,
        }
    };
    let samples = layout.row_bytes().checked_mul(height as usize)?;
    let pixels = layout.pixel_bytes()?;
    (samples <= MAX_SAMPLE_BYTES && pixels <= MAX_SAMPLE_BYTES).then_some(layout)
}

/// The pixels of the raw-sample image XObject `stream` in `doc`. Samples
/// shorter than the image are padded with zeros, as a viewer draws them.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the image is not of a supported kind (see
/// [`is_supported`]), its samples cannot be decompressed, or an index falls
/// outside its palette.
pub(crate) fn decode(doc: &Document, stream: &Stream) -> Result<Pixels> {
    let (layout, samples) = read(doc, stream)?;
    let row_bytes = layout.row_bytes();

    let count = layout.width as usize * layout.space.components();
    let mut data = Vec::with_capacity(layout.pixel_bytes().unwrap_or_default());
    for row in samples.chunks_exact(row_bytes) {
        let values = unpack(row, layout.bits, count);
        pixels(&layout.space, layout.bits, &values, &mut data)
            .ok_or_else(|| Error::malformed("image palette index out of range"))?;
    }
    Ok(Pixels {
        width: layout.width,
        height: layout.height,
        rgb: layout.space.rgb(),
        data,
    })
}

/// The samples of the raw-sample image XObject `stream` in `doc`, exactly as
/// many as the image takes: samples short of it padded with zeros, as a
/// viewer draws them, and bytes past it dropped, so nothing rides along
/// after the image. Every index of an indexed image is checked against its
/// palette, so samples kept are samples [`decode`] reads.
///
/// # Errors
///
/// As [`decode`].
pub(crate) fn samples(doc: &Document, stream: &Stream) -> Result<Vec<u8>> {
    let (layout, samples) = read(doc, stream)?;
    if let Space::Indexed { rgb, palette } = &layout.space {
        let colours = palette.len() / if *rgb { 3 } else { 1 };
        let count = layout.width as usize;
        let in_palette = samples.chunks_exact(layout.row_bytes()).all(|row| {
            unpack(row, layout.bits, count)
                .iter()
                .all(|&index| usize::from(index) < colours)
        });
        if !in_palette {
            return Err(Error::malformed("image palette index out of range"));
        }
    }
    Ok(samples)
}

/// The layout of `stream` and its samples, exactly as many as it takes.
fn read(doc: &Document, stream: &Stream) -> Result<(Layout, Vec<u8>)> {
    let layout =
        layout(doc, &stream.dict).ok_or_else(|| Error::malformed("unsupported image samples"))?;
    let size = layout.row_bytes() * layout.height as usize;
    let mut samples = stream
        .decompressed_content_with_limit(MAX_SAMPLE_BYTES)
        .map_err(|e| Error::malformed(format!("image samples: {e}")))?;
    samples.resize(size, 0);
    // The bits past a row's last sample, in its last byte, draw nothing, so
    // nothing is carried in them.
    let used = layout.width as usize * layout.space.components() * layout.bits;
    let spare = layout.row_bytes() * 8 - used;
    if spare > 0 {
        let mask = 0xFFu8 << spare;
        for row in samples.chunks_exact_mut(layout.row_bytes()) {
            if let Some(last) = row.last_mut() {
                *last &= mask;
            }
        }
    }
    Ok((layout, samples))
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
    let image = image.ok_or_else(|| Error::malformed("image pixels do not fill the image"))?;
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .map_err(|e| Error::malformed(format!("encode PNG: {e}")))?;
    Ok(out)
}

/// Whether the image's `/Decode` array inverts a gray sample, or `None` when
/// it maps samples some other non-default way, or is not an array of
/// numbers.
fn decode_array(doc: &Document, dict: &Dictionary) -> Option<bool> {
    let Ok(decode) = dict.get(b"Decode") else {
        return Some(false);
    };
    let values: Vec<f64> = doc
        .resolve_array(decode)?
        .iter()
        .map(|value| doc.resolve_number(value))
        .collect::<Option<_>>()?;
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
                    let profile = doc.resolve(array.get(1)?)?.as_stream().ok()?;
                    by_components(profile.dict.get(b"N").ok()?.as_i64().ok()?)
                }
                b"Indexed" | b"I" if !inverted => {
                    let base = space(doc, doc.resolve(array.get(1)?)?, Some(false))?;
                    let rgb = match base {
                        Space::Gray { .. } => false,
                        Space::Rgb => true,
                        _ => return None,
                    };
                    let hival = usize::try_from(doc.resolve(array.get(2)?)?.as_i64().ok()?)
                        .ok()
                        .filter(|&hival| hival <= 255)?;
                    let needed = (hival + 1) * if rgb { 3 } else { 1 };
                    // A palette has at most 256 entries, so a stream decoding
                    // past the colours they take is not read whole.
                    let palette = match doc.resolve(array.get(3)?)? {
                        Object::String(bytes, _) => bytes.clone(),
                        Object::Stream(stream) => stream
                            .decompressed_content_with_limit(MAX_PALETTE_BYTES)
                            .ok()?,
                        _ => return None,
                    };
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
            for [cyan, magenta, yellow, key] in values.as_chunks::<4>().0 {
                let k = u16::from(255 - scale(*key));
                for &ink in [cyan, magenta, yellow] {
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

    /// Samples are kept only when they decode: an index past the palette is
    /// refused when the samples are read, as when they are decoded.
    #[test]
    fn an_index_past_the_palette_is_refused() {
        let doc = Document::new();
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
            vec![1, 2],
        );
        assert!(decode(&doc, &indexed).is_err());
        assert!(samples(&doc, &indexed).is_err());
    }

    /// An image whose packed samples fit the bound but whose pixels do not is
    /// refused: 1-bit samples expand to a byte each.
    #[test]
    fn pixels_past_the_bound_are_refused() {
        let doc = Document::new();
        let mask = dictionary! { "Width" => 65_536, "Height" => 32_768, "ImageMask" => true };
        assert!(!is_supported(&doc, &mask));
        let gray = dictionary! {
            "Width" => 65_536, "Height" => 32_768, "BitsPerComponent" => 1,
            "ColorSpace" => "DeviceGray",
        };
        assert!(!is_supported(&doc, &gray));
    }

    /// A stencil mask reads as gray, its painted samples (0 under the
    /// default decode array) black; an inverted decode array swaps them.
    #[test]
    fn a_stencil_mask_reads_as_gray() {
        let doc = Document::new();
        let mask = dictionary! { "Width" => 2, "Height" => 1, "ImageMask" => true };
        assert!(is_supported(&doc, &mask));
        assert_eq!(
            decode(&doc, &image(mask, vec![0b0100_0000])).unwrap().data,
            [0, 255]
        );
        let inverted = dictionary! {
            "Width" => 2, "Height" => 1, "ImageMask" => true,
            "Decode" => vec![1.into(), 0.into()],
        };
        assert_eq!(
            decode(&doc, &image(inverted, vec![0b0100_0000]))
                .unwrap()
                .data,
            [255, 0]
        );
        let wide = dictionary! { "Width" => 2, "Height" => 1, "ImageMask" => true, "BitsPerComponent" => 8 };
        assert!(!is_supported(&doc, &wide));
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
