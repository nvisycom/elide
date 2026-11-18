//! Writing a font's `/ToUnicode` CMap.
//!
//! A `/ToUnicode` CMap maps character codes to Unicode, so text can be
//! recovered from it (copy, search, extraction) independently of the drawn
//! glyphs. A CMap is never edited as text: it is read as lopdf reads it, the
//! reader text detection decodes with, changed as a map of codes, and written
//! whole. So the CMap a viewer reads maps what detection read and nothing
//! else: no comment, and no entry lopdf passes over, survives.
//!
//! The rebuild writes each destination as [`readable`] text, so the text a
//! viewer copies is the text detection read; text redaction removes each
//! deleted code, so the redacted text cannot be recovered through the map.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use elide_core::{Error, Result};
use lopdf::{Document, Encoding, Object, ObjectId, Stream};

use crate::pdf::reader::font::{readable, to_unicode};

/// The most codes a CMap is read as. Each is held while the CMap is written,
/// and a range of four-byte codes may name billions.
const MAX_CODES: u64 = 1 << 18;

/// The most entries one `bfchar` or `bfrange` section holds.
const SECTION: usize = 100;

/// What a written CMap opens with: a Unicode CMap of its own name.
const HEADER: &str = "/CIDInit /ProcSet findresource begin\n\
    12 dict begin\n\
    begincmap\n\
    /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
    /CMapName /Adobe-Identity-UCS def\n\
    /CMapType 2 def\n";

/// What a written CMap closes with.
const FOOTER: &str = "endcmap\n\
    CMapName currentdict /CMap defineresource pop\n\
    end\n\
    end\n";

/// A `/ToUnicode` CMap as lopdf reads it: each code, by its length in bytes,
/// and the UTF-16 units it maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnicodeMap {
    codes: BTreeMap<(u8, u32), Vec<u16>>,
}

impl UnicodeMap {
    /// The CMap `stream` in `doc` as lopdf reads it, or `None` when lopdf
    /// can't read it.
    ///
    /// # Errors
    ///
    /// As [`to_unicode`], and
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// it maps more than [`MAX_CODES`] codes.
    pub(crate) fn read(doc: &Document, stream: &Stream) -> Result<Option<Self>> {
        let Some(Encoding::UnicodeMapEncoding(cmap)) = to_unicode(doc, stream)? else {
            return Ok(None);
        };
        let count: u64 = cmap
            .bf_ranges
            .iter()
            .flat_map(|ranges| ranges.iter())
            .map(|(range, _)| u64::from(range.end() - range.start()) + 1)
            .sum();
        if count > MAX_CODES {
            return Err(Error::resource_limit(format!(
                "a /ToUnicode CMap maps more than {MAX_CODES} codes"
            )));
        }
        let mut codes = BTreeMap::new();
        for (len, ranges) in (1u8..).zip(&cmap.bf_ranges) {
            for (range, _) in ranges.iter() {
                for code in range.clone() {
                    if let Some(units) = cmap.get(code, len).filter(|units| !units.is_empty()) {
                        codes.insert((len, code), units);
                    }
                }
            }
        }
        Ok(Some(Self { codes }))
    }

    /// Each destination written as [`readable`] text; one that is not UTF-16
    /// stays as it is.
    pub(crate) fn readable(mut self) -> Self {
        for units in self.codes.values_mut() {
            if let Ok(text) = String::from_utf16(units) {
                let written = readable(&text);
                if written != text {
                    *units = written.encode_utf16().collect();
                }
            }
        }
        self
    }

    /// Unmap the code text draws as `bytes`.
    pub(crate) fn remove(&mut self, bytes: &[u8]) {
        if let Some(key) = key(bytes) {
            self.codes.remove(&key);
        }
    }

    /// The CMap written: a codespace for each code length, then each run of
    /// codes whose destinations step one by one as a `bfrange`, and every
    /// other code as a `bfchar`.
    pub(crate) fn write(&self) -> Vec<u8> {
        let mut out = String::from(HEADER);

        // Each length's codespace spans, byte by byte, the codes it maps.
        let mut spaces: BTreeMap<u8, (u32, u32)> = BTreeMap::new();
        for &(len, code) in self.codes.keys() {
            let (low, high) = spaces.entry(len).or_insert((code, code));
            for shift in (0..len).map(|byte| 8 * u32::from(byte)) {
                let byte = (code >> shift) & 0xFF;
                *low = with_byte(*low, shift, byte.min((*low >> shift) & 0xFF));
                *high = with_byte(*high, shift, byte.max((*high >> shift) & 0xFF));
            }
        }
        let _ = writeln!(out, "{} begincodespacerange", spaces.len());
        for (&len, &(low, high)) in &spaces {
            let _ = writeln!(out, "{} {}", code_hex(len, low), code_hex(len, high));
        }
        out.push_str("endcodespacerange\n");

        // A run stays within one last-byte block, as a range's bounds differ
        // only in their last byte.
        let mut runs: Vec<(u8, u32, u32, &[u16])> = Vec::new();
        for (&(len, code), units) in &self.codes {
            if let Some((run_len, low, high, first)) = runs.last_mut()
                && *run_len == len
                && *high + 1 == code
                && code & 0xFF != 0
                && steps(first, code - *low, units)
            {
                *high = code;
                continue;
            }
            runs.push((len, code, code, units));
        }
        let (chars, ranges): (Vec<_>, Vec<_>) =
            runs.into_iter().partition(|&(_, low, high, _)| low == high);
        for section in chars.chunks(SECTION) {
            let _ = writeln!(out, "{} beginbfchar", section.len());
            for &(len, code, _, units) in section {
                let _ = writeln!(out, "{} {}", code_hex(len, code), units_hex(units));
            }
            out.push_str("endbfchar\n");
        }
        for section in ranges.chunks(SECTION) {
            let _ = writeln!(out, "{} beginbfrange", section.len());
            for &(len, low, high, units) in section {
                let _ = writeln!(
                    out,
                    "{} {} {}",
                    code_hex(len, low),
                    code_hex(len, high),
                    units_hex(units)
                );
            }
            out.push_str("endbfrange\n");
        }
        out.push_str(FOOTER);
        out.into_bytes()
    }
}

/// Remove `deleted` codes from each named `/ToUnicode` CMap stream, sparing
/// any code `surviving` text still draws with it.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a CMap
/// holding a deleted code cannot be read, so the redaction fails closed
/// rather than leaving a recoverable code->Unicode entry; or as
/// [`UnicodeMap::read`].
pub(crate) fn scrub(
    doc: &mut Document,
    deleted: &BTreeMap<ObjectId, BTreeSet<Vec<u8>>>,
    surviving: &BTreeMap<ObjectId, BTreeSet<Vec<u8>>>,
) -> Result<()> {
    for (&cmap_id, deleted_here) in deleted {
        // Fonts and their CMaps are shared, and a code (the letter `a`)
        // recurs in text that stays: only codes used nowhere else go.
        let empty = BTreeSet::new();
        let kept = surviving.get(&cmap_id).unwrap_or(&empty);
        let codes: Vec<&Vec<u8>> = deleted_here.difference(kept).collect();
        if codes.is_empty() {
            continue;
        }
        let Ok(Object::Stream(stream)) = doc.get_object(cmap_id) else {
            continue;
        };
        let Some(mut map) = UnicodeMap::read(doc, stream)? else {
            return Err(Error::redaction(
                "a /ToUnicode CMap mapping deleted glyphs cannot be read, so their \
                 codes cannot be removed from it",
            ));
        };
        for code in codes {
            map.remove(code);
        }
        if let Ok(Object::Stream(stream)) = doc.get_object_mut(cmap_id) {
            stream.set_plain_content(map.write());
        }
    }
    Ok(())
}

/// The code `bytes` name, by its length, or `None` past four bytes.
fn key(bytes: &[u8]) -> Option<(u8, u32)> {
    let len = u8::try_from(bytes.len())
        .ok()
        .filter(|len| (1..=4).contains(len))?;
    Some((
        len,
        bytes.iter().fold(0, |code, &b| code << 8 | u32::from(b)),
    ))
}

/// Whether `units` are what a range starting at `first` maps the code
/// `step` past its first to: `first` with its last unit stepped.
fn steps(first: &[u16], step: u32, units: &[u16]) -> bool {
    let (Some((&last, head)), Some((&unit, rest))) = (first.split_last(), units.split_last())
    else {
        return false;
    };
    head == rest && u32::from(last) + step == u32::from(unit)
}

/// `code` with its byte at `shift` set to `byte`.
fn with_byte(code: u32, shift: u32, byte: u32) -> u32 {
    code & !(0xFF << shift) | byte << shift
}

/// A `len`-byte `code` as a `<hex>` string.
fn code_hex(len: u8, code: u32) -> String {
    format!("<{code:0width$X}>", width = 2 * usize::from(len))
}

/// UTF-16 `units` as a `<hex>` string.
fn units_hex(units: &[u16]) -> String {
    let digits: String = units.iter().map(|unit| format!("{unit:04X}")).collect();
    format!("<{digits}>")
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use lopdf::dictionary;

    use super::*;

    /// `cmap` read as lopdf reads it.
    fn read(cmap: &[u8]) -> UnicodeMap {
        let stream = Stream::new(dictionary! {}, cmap.to_vec());
        UnicodeMap::read(&Document::new(), &stream)
            .unwrap()
            .unwrap()
    }

    /// A CMap of `sections`, in the shape lopdf reads.
    fn cmap(sections: &str) -> Vec<u8> {
        format!("{HEADER}{sections}\n{FOOTER}").into_bytes()
    }

    /// The text `map` maps the `len`-byte `code` to.
    fn text(map: &UnicodeMap, len: u8, code: u32) -> Option<String> {
        map.codes
            .get(&(len, code))
            .map(|units| String::from_utf16(units).unwrap())
    }

    /// A CMap as a producer writes one: chars, a stepping range, and a range
    /// of an array.
    fn source() -> Vec<u8> {
        cmap(
            "1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
            2 beginbfchar\n<0003> <0041>\n<0004> <00660069>\nendbfchar\n\
            2 beginbfrange\n<0010> <0012> <0061>\n<0020> <0021> [<2F2D> <0042>]\nendbfrange",
        )
    }

    #[test]
    fn a_written_map_reads_back_the_same() {
        let map = read(&source());
        assert_eq!(text(&map, 2, 0x11).as_deref(), Some("b"));
        assert_eq!(text(&map, 2, 0x21).as_deref(), Some("B"));
        assert_eq!(read(&map.write()), map);
    }

    /// Codes whose destinations step one by one are written as one range.
    #[test]
    fn stepping_codes_are_written_as_a_range() {
        let written = String::from_utf8(read(&source()).write()).unwrap();
        assert!(written.contains("<0010> <0012> <0061>"), "{written}");
        assert!(written.contains("<0004> <00660069>"), "{written}");
    }

    /// Each destination reads as the text detection reads: a radical as its
    /// ideograph, a ligature as its letters.
    #[test]
    fn destinations_are_written_as_readable_text() {
        let map = read(&source()).readable();
        assert_eq!(text(&map, 2, 0x20).as_deref(), Some("山"));
        assert_eq!(text(&map, 2, 0x04).as_deref(), Some("fi"));
        assert_eq!(read(&map.write()), map);
    }

    /// A code removed from inside a range leaves the codes around it
    /// mapped as they were.
    #[test]
    fn a_code_is_removed_from_inside_a_range() {
        let mut map = read(&source());
        map.remove(&[0x00, 0x11]);
        let map = read(&map.write());
        assert_eq!(text(&map, 2, 0x10).as_deref(), Some("a"));
        assert_eq!(text(&map, 2, 0x11), None);
        assert_eq!(text(&map, 2, 0x12).as_deref(), Some("c"));
    }

    /// Nothing but the map is written: a comment does not survive, and an
    /// entry in it is no entry.
    #[test]
    fn only_the_map_is_written() {
        let map = read(&cmap(
            "1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
            1 beginbfchar\n% <01> Jane Roe\n<02> <0041>\nendbfchar",
        ));
        assert_eq!(text(&map, 1, 0x01), None);
        let written = String::from_utf8(map.write()).unwrap();
        assert!(!written.contains("Jane"));
        assert_eq!(
            text(&read(written.as_bytes()), 1, 0x02).as_deref(),
            Some("A")
        );
    }

    /// A CMap naming more codes than are held is refused before any is.
    #[test]
    fn a_map_of_too_many_codes_is_refused() {
        let stream = Stream::new(
            dictionary! {},
            cmap(
                "1 begincodespacerange\n<000000> <FFFFFF>\nendcodespacerange\n\
                1 beginbfrange\n<000000> <04FFFF> <0041>\nendbfrange",
            ),
        );
        let err = UnicodeMap::read(&Document::new(), &stream).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ResourceLimit);
    }
}
