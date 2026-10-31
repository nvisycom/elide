//! Container-level metadata scrub: drop every metadata segment of a JPEG or
//! PNG container except what a viewer needs to render the image.
//!
//! EXIF is only one of the metadata blocks an image carries. A JPEG also holds
//! XMP (a second APP1), IPTC and Photoshop resources (APP13), Ducky (APP12),
//! and free-text `COM` comments; a PNG holds `tEXt`, `zTXt`, and `iTXt` chunks
//! (author, description, XMP) and a `tIME` stamp. None of these surface as
//! fields, so they are removed wholesale. The scrub is an allowlist: a segment
//! is kept only when it is known to be needed for rendering (or is the EXIF
//! block itself, when the caller keeps it for field-level redaction), so a
//! vendor or future metadata segment cannot slip through.

use elide_core::{Error, ErrorKind, Result};

/// The JPEG `SOI` marker that opens every JPEG stream.
const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];

/// The 8-byte signature that opens every PNG stream.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// The ancillary PNG chunks kept by the scrub: transparency, colour space and
/// profile, gamma, significant bits, background, pixel density, HDR metadata,
/// and the animation frames of an APNG. Critical chunks (an uppercase first
/// letter) are always kept.
const PNG_RENDER_CHUNKS: [&[u8; 4]; 14] = [
    b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT", b"bKGD", b"pHYs", b"cICP", b"mDCv",
    b"cLLi", b"acTL", b"fcTL", b"fdAT",
];

/// A container with metadata dropped: the remaining bytes, how many metadata
/// blocks or tags (JPEG segments, PNG chunks, EXIF and TIFF tags) were dropped,
/// and how many embedded thumbnails.
#[derive(Debug)]
pub(crate) struct Scrubbed {
    pub(crate) bytes: Vec<u8>,
    pub(crate) removed: usize,
    pub(crate) thumbnails: usize,
}

/// The keyword of the PNG text chunk some writers store raw EXIF in.
const PNG_RAW_EXIF_KEYWORD: &[u8] = b"Raw profile type exif";

/// Drop every metadata segment of a JPEG but the ones that render it (`JFIF`
/// in APP0, the ICC profile in APP2, the Adobe colour transform in APP14),
/// keeping the EXIF APP1 too when `keep_exif` holds.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the bytes are not a well-formed JPEG
/// header.
pub(super) fn scrub_jpeg(bytes: &[u8], keep_exif: bool) -> Result<Scrubbed> {
    if !bytes.starts_with(&JPEG_SOI) {
        return Err(malformed("not a JPEG stream"));
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&JPEG_SOI);
    let mut removed = 0;
    let mut at = JPEG_SOI.len();
    loop {
        // Fill bytes (repeated 0xFF) may precede a marker.
        while bytes.get(at) == Some(&0xFF) && bytes.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        let (Some(&0xFF), Some(&marker)) = (bytes.get(at), bytes.get(at + 1)) else {
            return Err(malformed("JPEG segment does not start with a marker"));
        };
        // Standalone markers carry no length: TEM and the restart markers.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            out.extend_from_slice(&bytes[at..at + 2]);
            at += 2;
            continue;
        }
        // End of image before any scan, or the start of scan: the entropy-coded
        // data and everything after it carry no metadata segments.
        if marker == 0xD9 || marker == 0xDA {
            out.extend_from_slice(&bytes[at..]);
            return Ok(Scrubbed {
                bytes: out,
                removed,
                thumbnails: 0,
            });
        }
        let length = bytes
            .get(at + 2..at + 4)
            .map(|len| usize::from(u16::from_be_bytes([len[0], len[1]])))
            .ok_or_else(|| malformed("truncated JPEG segment length"))?;
        let end = at + 2 + length;
        if length < 2 || end > bytes.len() {
            return Err(malformed("JPEG segment overruns the stream"));
        }
        let payload = &bytes[at + 4..end];
        if keeps_jpeg_segment(marker, payload, keep_exif) {
            out.extend_from_slice(&bytes[at..end]);
        } else {
            removed += 1;
        }
        at = end;
    }
}

/// Whether the JPEG segment `marker` with `payload` survives the scrub. Every
/// non-APPn, non-COM segment (tables, frame headers) is structural and kept.
fn keeps_jpeg_segment(marker: u8, payload: &[u8], keep_exif: bool) -> bool {
    match marker {
        0xE0 => payload.starts_with(b"JFIF\0") || payload.starts_with(b"JFXX\0"),
        0xE1 => keep_exif && payload.starts_with(b"Exif\0\0"),
        0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
        0xEE => payload.starts_with(b"Adobe"),
        0xE3..=0xED | 0xEF | 0xFE => false,
        _ => true,
    }
}

/// Drop every ancillary chunk of a PNG but the ones that render it, keeping the
/// EXIF chunk (`eXIf`, or a raw-EXIF text chunk) too when `keep_exif` holds.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the bytes are not a well-formed PNG chunk
/// stream.
pub(super) fn scrub_png(bytes: &[u8], keep_exif: bool) -> Result<Scrubbed> {
    if !bytes.starts_with(&PNG_SIGNATURE) {
        return Err(malformed("not a PNG stream"));
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&PNG_SIGNATURE);
    let mut removed = 0;
    let mut at = PNG_SIGNATURE.len();
    while at < bytes.len() {
        let header = bytes
            .get(at..at + 8)
            .ok_or_else(|| malformed("truncated PNG chunk header"))?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind: &[u8; 4] = header[4..8].try_into().expect("four bytes");
        // Length, type, data, CRC.
        let end = at
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| malformed("PNG chunk overruns the stream"))?;
        let data = &bytes[at + 8..end - 4];
        if keeps_png_chunk(kind, data, keep_exif) {
            out.extend_from_slice(&bytes[at..end]);
        } else {
            removed += 1;
        }
        at = end;
        if kind == b"IEND" {
            break;
        }
    }
    Ok(Scrubbed {
        bytes: out,
        removed,
        thumbnails: 0,
    })
}

/// Whether the PNG chunk `kind` with `data` survives the scrub.
fn keeps_png_chunk(kind: &[u8; 4], data: &[u8], keep_exif: bool) -> bool {
    // Critical chunks (IHDR, PLTE, IDAT, IEND) have an uppercase first letter.
    if kind[0].is_ascii_uppercase() || PNG_RENDER_CHUNKS.contains(&kind) {
        return true;
    }
    if !keep_exif {
        return false;
    }
    match kind {
        b"eXIf" => true,
        b"zTXt" | b"tEXt" | b"iTXt" => data.starts_with(PNG_RAW_EXIF_KEYWORD),
        _ => false,
    }
}

/// A malformed-container error with `message`.
fn malformed(message: &str) -> Error {
    Error::new(
        ErrorKind::MalformedInput,
        format!("image metadata scrub: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG segment: marker, big-endian length (payload + 2), payload.
    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, marker];
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// A JPEG of `segments` between SOI and a minimal scan.
    fn jpeg(segments: &[Vec<u8>]) -> Vec<u8> {
        let mut out = JPEG_SOI.to_vec();
        for s in segments {
            out.extend_from_slice(s);
        }
        out.extend_from_slice(&segment(0xDA, b"scan"));
        out.extend_from_slice(&[0x12, 0x34, 0xFF, 0xD9]);
        out
    }

    /// A PNG chunk with a zeroed CRC (the scrub copies chunks, never checks it).
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0; 4]);
        out
    }

    /// A PNG of `chunks` between the signature and IEND.
    fn png(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        out.extend_from_slice(&chunk(b"IHDR", &[0; 13]));
        for c in chunks {
            out.extend_from_slice(c);
        }
        out.extend_from_slice(&chunk(b"IDAT", b"pixels"));
        out.extend_from_slice(&chunk(b"IEND", &[]));
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn jpeg_scrub_drops_xmp_iptc_and_comments_and_keeps_render_segments() {
        let bytes = jpeg(&[
            segment(0xE0, b"JFIF\0rest"),
            segment(0xE1, b"Exif\0\0exif-block"),
            segment(
                0xE1,
                b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>Jane</x:xmpmeta>",
            ),
            segment(0xE2, b"ICC_PROFILE\0profile"),
            segment(0xED, b"Photoshop 3.0\0iptc"),
            segment(0xFE, b"taken by Jane"),
            segment(0xDB, b"quant-table"),
        ]);
        let out = scrub_jpeg(&bytes, false).unwrap().bytes;
        assert!(contains(&out, b"JFIF"));
        assert!(contains(&out, b"ICC_PROFILE"));
        assert!(contains(&out, b"quant-table"));
        assert!(contains(&out, b"scan"));
        assert!(!contains(&out, b"exif-block"));
        assert!(!contains(&out, b"Jane"));
        assert!(!contains(&out, b"iptc"));
        assert!(out.ends_with(&[0x12, 0x34, 0xFF, 0xD9]));
    }

    #[test]
    fn jpeg_scrub_keeps_only_the_exif_block_when_asked() {
        let bytes = jpeg(&[
            segment(0xE1, b"Exif\0\0exif-block"),
            segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0xmp"),
        ]);
        let out = scrub_jpeg(&bytes, true).unwrap().bytes;
        assert!(contains(&out, b"exif-block"));
        assert!(!contains(&out, b"xap"));
    }

    #[test]
    fn jpeg_scrub_rejects_a_truncated_segment() {
        let mut bytes = JPEG_SOI.to_vec();
        bytes.extend_from_slice(&[0xFF, 0xE1, 0x00, 0x40, b'E']);
        assert!(scrub_jpeg(&bytes, false).is_err());
        assert!(scrub_jpeg(b"not a jpeg", false).is_err());
    }

    #[test]
    fn png_scrub_drops_text_and_time_and_keeps_render_chunks() {
        let bytes = png(&[
            chunk(b"iCCP", b"profile"),
            chunk(b"tEXt", b"Author\0Jane Roe"),
            chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<xmp/>"),
            chunk(b"tIME", &[0; 7]),
            chunk(b"eXIf", b"exif-block"),
            chunk(b"pHYs", &[0; 9]),
        ]);
        let out = scrub_png(&bytes, false).unwrap().bytes;
        assert!(contains(&out, b"iCCP"));
        assert!(contains(&out, b"pHYs"));
        assert!(contains(&out, b"IDAT"));
        assert!(contains(&out, b"IEND"));
        assert!(!contains(&out, b"Jane"));
        assert!(!contains(&out, b"xmp"));
        assert!(!contains(&out, b"tIME"));
        assert!(!contains(&out, b"exif-block"));
    }

    #[test]
    fn png_scrub_keeps_only_the_exif_chunks_when_asked() {
        let bytes = png(&[
            chunk(b"eXIf", b"exif-block"),
            chunk(b"zTXt", b"Raw profile type exif\0\0raw"),
            chunk(b"tEXt", b"Comment\0hello"),
        ]);
        let out = scrub_png(&bytes, true).unwrap().bytes;
        assert!(contains(&out, b"exif-block"));
        assert!(contains(&out, b"Raw profile type exif"));
        assert!(!contains(&out, b"hello"));
    }

    #[test]
    fn png_scrub_rejects_an_overrunning_chunk() {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 99, b't', b'E', b'X', b't']);
        assert!(scrub_png(&bytes, false).is_err());
        assert!(scrub_png(b"not a png", false).is_err());
    }
}
