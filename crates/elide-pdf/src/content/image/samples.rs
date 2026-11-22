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

use crate::pdf::reader::{DictionaryExt, DocumentExt};

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

    /// An indexed space's last index (its `hival`), `None` for any other.
    fn last_index(&self) -> Option<u8> {
        let Self::Indexed { rgb, palette } = self else {
            return None;
        };
        let colours = palette.len() / if *rgb { 3 } else { 1 };
        u8::try_from(colours.checked_sub(1)?).ok()
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
    dict.resolved(doc, b"ImageMask")
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
    let width = u32::try_from(integer(doc, dict, b"Width")?).ok()?;
    let height = u32::try_from(integer(doc, dict, b"Height")?).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let (bits, space) = if is_stencil(doc, dict) {
        stencil_samples(doc, dict)?
    } else {
        colour_samples(doc, dict)?
    };
    let layout = Layout {
        width,
        height,
        bits,
        space,
    };
    let samples = layout.row_bytes().checked_mul(height as usize)?;
    let pixels = layout.pixel_bytes()?;
    (samples <= MAX_SAMPLE_BYTES && pixels <= MAX_SAMPLE_BYTES).then_some(layout)
}

/// The integer `dict` holds under `key`.
fn integer(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    dict.resolved(doc, key)?.as_i64().ok()
}

/// The bits and space of the stencil mask `dict`'s samples: 1 bit each, a 0
/// painting under the default decode array (8.9.6.2), so as gray the painted
/// samples are black.
fn stencil_samples(doc: &Document, dict: &Dictionary) -> Option<(usize, Space)> {
    if integer(doc, dict, b"BitsPerComponent").unwrap_or(1) != 1 {
        return None;
    }
    let inverted = decode_array(doc, dict)?;
    Some((1, Space::Gray { inverted }))
}

/// The bits and space of the colour image `dict`'s samples. An index's
/// decode array is `[0 2^bits-1]` unless it says otherwise (8.9.5.2), not a
/// colour's `[0 1]`; one that maps indexes elsewhere is not read.
fn colour_samples(doc: &Document, dict: &Dictionary) -> Option<(usize, Space)> {
    let bits = integer(doc, dict, b"BitsPerComponent")?;
    if ![1, 2, 4, 8].contains(&bits) {
        return None;
    }
    let colours = dict.resolved(doc, b"ColorSpace")?;
    let indexed = colours.as_array().is_ok_and(|array| {
        array
            .first()
            .and_then(|family| doc.resolve(family)?.as_name().ok())
            .is_some_and(|family| matches!(family, b"Indexed" | b"I"))
    });
    let inverted = if indexed {
        Some(false)
    } else {
        decode_array(doc, dict)
    };
    let space = space(doc, colours, inverted)?;
    if indexed && let Ok(decode) = dict.get(b"Decode") {
        let identity = [0.0, ((1u32 << bits) - 1) as f64];
        let values: Option<Vec<f64>> = doc
            .resolve_array(decode)
            .and_then(|values| values.iter().map(|v| doc.resolve_number(v)).collect());
        if values.as_deref() != Some(&identity[..]) {
            return None;
        }
    }
    Some((bits as usize, space))
}

/// The pixels of the raw-sample image XObject `stream` in `doc`. Samples
/// shorter than the image are padded with zeros, as a viewer draws them.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the image is not of a supported kind (see
/// [`is_supported`]), or its samples cannot be decompressed.
pub(crate) fn decode(doc: &Document, stream: &Stream) -> Result<Pixels> {
    let (layout, samples) = read(doc, stream)?;
    let row_bytes = layout.row_bytes();

    let count = layout.width as usize * layout.space.components();
    let mut data = Vec::with_capacity(layout.pixel_bytes().unwrap_or_default());
    for row in samples.chunks_exact(row_bytes) {
        let values = unpack(row, layout.bits, count);
        pixels(&layout.space, layout.bits, &values, &mut data);
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
/// after the image. An indexed image's index past its last colour is
/// written as that colour's, as a viewer clamps it (8.9.5.2), so the
/// samples kept are the colours drawn, unless a colour key masks the image
/// by its raw samples.
///
/// # Errors
///
/// As [`decode`].
pub(crate) fn samples(doc: &Document, stream: &Stream) -> Result<Vec<u8>> {
    let (layout, mut samples) = read(doc, stream)?;
    // A colour key masks by the raw sample, before any clamp (8.9.6.4), so
    // a keyed image's samples are kept as they are.
    let keyed = stream
        .dict
        .resolved(doc, b"Mask")
        .is_some_and(|mask| matches!(mask, Object::Array(_)));
    if !keyed && let Some(last) = layout.space.last_index() {
        let count = layout.width as usize * layout.space.components();
        for row in samples.chunks_exact_mut(layout.row_bytes()) {
            let values = unpack(row, layout.bits, count);
            if values.iter().any(|&index| index > last) {
                let clamped: Vec<u8> = values.iter().map(|&index| index.min(last)).collect();
                pack(&clamped, layout.bits, row);
            }
        }
    }
    Ok(samples)
}

/// Where the stencil mask `stream` in `doc` paints, as a box `[x0 y0 x1 y1]`
/// in its unit square (y up, its first row at the top): around the samples
/// its decode array paints, `None` when it paints none, and the whole square
/// when its samples cannot be read.
pub(crate) fn stencil_extent(doc: &Document, stream: &Stream) -> Option<[f64; 4]> {
    let Ok((layout, samples)) = read(doc, stream) else {
        return Some([0.0, 0.0, 1.0, 1.0]);
    };
    let Space::Gray { inverted } = layout.space else {
        return Some([0.0, 0.0, 1.0, 1.0]);
    };
    // A 0 paints under the default decode array, a 1 under `[1 0]`.
    let paints = u8::from(inverted);
    let width = layout.width as usize;
    let mut extent: Option<(usize, usize, usize, usize)> = None;
    for (row, bits) in samples.chunks_exact(layout.row_bytes()).enumerate() {
        let values = unpack(bits, layout.bits, width);
        let (Some(first), Some(last)) = (
            values.iter().position(|&v| v == paints),
            values.iter().rposition(|&v| v == paints),
        ) else {
            continue;
        };
        extent = Some(match extent {
            None => (first, row, last, row),
            Some((x0, y0, x1, _)) => (x0.min(first), y0, x1.max(last), row),
        });
    }
    let (x0, first_row, x1, last_row) = extent?;
    let (w, h) = (f64::from(layout.width), f64::from(layout.height));
    Some([
        x0 as f64 / w,
        1.0 - (last_row + 1) as f64 / h,
        (x1 + 1) as f64 / w,
        1.0 - first_row as f64 / h,
    ])
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
                    // Only the colours up to `hival` are the palette's.
                    let palette = palette.get(..needed)?.to_vec();
                    Some(Space::Indexed { rgb, palette })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Write `values` into `row` as `bits`-per-sample samples, from its start;
/// the bits past them stay as they are.
fn pack(values: &[u8], bits: usize, row: &mut [u8]) {
    if bits == 8 {
        row[..values.len()].copy_from_slice(values);
        return;
    }
    let per_byte = 8 / bits;
    let mask = (1u8 << bits) - 1;
    for (i, &value) in values.iter().enumerate() {
        let shift = 8 - bits * (i % per_byte + 1);
        let byte = &mut row[i / per_byte];
        *byte = *byte & !(mask << shift) | (value & mask) << shift;
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

/// Append the pixels of one row of sample `values` in `space` to `out`; an
/// index past the palette's last colour is that colour (8.9.5.2).
fn pixels(space: &Space, bits: usize, values: &[u8], out: &mut Vec<u8>) {
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
            let last = usize::from(space.last_index().unwrap_or(0));
            for &index in values {
                let at = usize::from(index).min(last) * width;
                out.extend_from_slice(&palette[at..at + width]);
            }
        }
    }
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
                    Object::from("Indexed"),
                    Object::from("DeviceRGB"),
                    1.into(),
                    Object::String(vec![255, 0, 0, 0, 0, 255], StringFormat::Hexadecimal),
                ],
            },
            vec![1, 0],
        );
        assert_eq!(decode(&doc, &indexed).unwrap().data, [0, 0, 255, 255, 0, 0]);
    }

    /// An index past the palette's last colour is that colour, as a viewer
    /// clamps it: in the samples kept, and in the pixels decoded.
    #[test]
    fn an_index_past_the_palette_is_clamped() {
        let doc = Document::new();
        let indexed = image(
            dictionary! {
                "Width" => 2, "Height" => 1, "BitsPerComponent" => 8,
                "ColorSpace" => vec![
                    Object::from("Indexed"),
                    Object::from("DeviceRGB"),
                    1.into(),
                    Object::String(vec![255, 0, 0, 0, 0, 255], StringFormat::Hexadecimal),
                ],
            },
            vec![1, 2],
        );
        assert_eq!(decode(&doc, &indexed).unwrap().data, [0, 0, 255, 0, 0, 255]);
        assert_eq!(samples(&doc, &indexed).unwrap(), [1, 1]);
        let packed = image(
            dictionary! {
                "Width" => 4, "Height" => 1, "BitsPerComponent" => 2,
                "ColorSpace" => vec![
                    Object::from("Indexed"),
                    Object::from("DeviceGray"),
                    1.into(),
                    Object::String(vec![0, 255], StringFormat::Hexadecimal),
                ],
            },
            vec![0b00_01_10_11],
        );
        assert_eq!(samples(&doc, &packed).unwrap(), [0b00_01_01_01]);

        // A colour key reads the raw sample, so a keyed image keeps it.
        let mut keyed = indexed.clone();
        keyed.dict.set("Mask", vec![1.into(), 1.into()]);
        assert_eq!(samples(&doc, &keyed).unwrap(), [1, 2]);
        // An index decode array other than the identity is not read.
        let mut decoded = indexed.clone();
        decoded.dict.set("Decode", vec![0.into(), 1.into()]);
        assert!(samples(&doc, &decoded).is_err());
        decoded.dict.set("Decode", vec![0.into(), 255.into()]);
        assert!(samples(&doc, &decoded).is_ok());
    }

    /// A stencil paints where its samples paint: around them, nothing when
    /// none does, and the inverse samples under `[1 0]`.
    #[test]
    fn a_stencil_paints_where_its_samples_do() {
        let doc = Document::new();
        let stencil = |samples: Vec<u8>, decode: Option<Vec<Object>>| {
            let mut dict = dictionary! { "Width" => 2, "Height" => 2, "ImageMask" => true };
            if let Some(decode) = decode {
                dict.set("Decode", decode);
            }
            image(dict, samples)
        };
        // Row 0 (the top) paints its right sample; row 1 paints nothing.
        let extent = stencil_extent(&doc, &stencil(vec![0b1000_0000, 0b1100_0000], None));
        assert_eq!(extent, Some([0.5, 0.5, 1.0, 1.0]));
        assert_eq!(stencil_extent(&doc, &stencil(vec![0xC0, 0xC0], None)), None);
        let inverted = stencil(vec![0xC0, 0xC0], Some(vec![1.into(), 0.into()]));
        assert_eq!(stencil_extent(&doc, &inverted), Some([0.0, 0.0, 1.0, 1.0]));
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
