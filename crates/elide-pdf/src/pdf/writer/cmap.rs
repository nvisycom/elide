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
use lopdf::{Dictionary, Document, Encoding, Object, ObjectId, Stream};

use crate::pdf::reader::font::{readable, to_unicode};

/// The most UTF-16 units a CMap's destinations hold together, so the map
/// held while a CMap is written, and the CMap written, stay bounded.
const MAX_UNITS: usize = 1 << 20;

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
    /// its destinations hold more than [`MAX_UNITS`] UTF-16 units together.
    pub(crate) fn read(doc: &Document, stream: &Stream) -> Result<Option<Self>> {
        let Some(Encoding::UnicodeMapEncoding(cmap)) = to_unicode(doc, stream)? else {
            return Ok(None);
        };
        let mut codes = BTreeMap::new();
        let mut units_left = MAX_UNITS;
        for (len, ranges) in (1u8..).zip(&cmap.bf_ranges) {
            for (range, _) in ranges.iter() {
                for code in range.clone() {
                    let Some(units) = cmap.get(code, len).filter(|units| !units.is_empty()) else {
                        continue;
                    };
                    units_left = units_left.checked_sub(units.len()).ok_or_else(|| {
                        Error::resource_limit(format!(
                            "a /ToUnicode CMap maps to more than {MAX_UNITS} UTF-16 units"
                        ))
                    })?;
                    codes.insert((len, code), units);
                }
            }
        }
        Ok(Some(Self { codes }))
    }

    /// Each destination written as [`readable`] text; one that is not UTF-16
    /// stays as it is.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// the destinations, so written, hold more than [`MAX_UNITS`] UTF-16
    /// units together, so the map written would not be read again.
    pub(crate) fn readable(mut self) -> Result<Self> {
        let mut units_left = MAX_UNITS;
        for units in self.codes.values_mut() {
            if let Ok(text) = String::from_utf16(units) {
                let written = readable(&text);
                if written != text {
                    *units = written.encode_utf16().collect();
                }
            }
            units_left = units_left.checked_sub(units.len()).ok_or_else(|| {
                Error::resource_limit(format!(
                    "a /ToUnicode CMap maps to more than {MAX_UNITS} UTF-16 units"
                ))
            })?;
        }
        Ok(self)
    }

    /// Unmap every code of one of `values`, at any length: the code text
    /// drew, and every entry a viewer reading a code by its value alone would
    /// take for it; an entry `kept` (codes text that stays still draws, by
    /// their bytes) is spared.
    pub(crate) fn remove(&mut self, values: &BTreeSet<u32>, kept: &BTreeSet<Vec<u8>>) {
        self.codes
            .retain(|&(len, code), _| !removed(len, code, values, kept));
    }

    /// Whether any entry maps a code [`remove`](Self::remove) would remove.
    fn maps(&self, values: &BTreeSet<u32>, kept: &BTreeSet<Vec<u8>>) -> bool {
        self.codes
            .keys()
            .any(|&(len, code)| removed(len, code, values, kept))
    }

    /// The CMap written: its codespace, then each run of codes whose
    /// destinations step one by one as a `bfrange`, and every other code as
    /// a `bfchar`.
    pub(crate) fn write(&self) -> Vec<u8> {
        let mut out = String::from(HEADER);
        for section in self.codespace().chunks(SECTION) {
            let _ = writeln!(out, "{} begincodespacerange", section.len());
            for &(len, low, high) in section {
                let _ = writeln!(out, "{} {}", code_hex(len, low), code_hex(len, high));
            }
            out.push_str("endcodespacerange\n");
        }

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

impl UnicodeMap {
    /// The codespace of the codes mapped: a range for each run of first
    /// bytes, in order, that codes of one length start with, its other bytes
    /// spanning those of the length's codes. A run ends only where a code of
    /// another length starts between, so codes of different lengths keep the
    /// first bytes they have apart and a reader splits them as the source
    /// did. A map of no code spans the one-byte codes, so the CMap stays one a
    /// reader reads.
    fn codespace(&self) -> Vec<(u8, u32, u32)> {
        let mut leads: BTreeSet<(u32, u8)> = BTreeSet::new();
        let mut rest: BTreeMap<u8, (u32, u32)> = BTreeMap::new();
        for &(len, code) in self.codes.keys() {
            let shift = 8 * (u32::from(len) - 1);
            leads.insert((code >> shift, len));
            let tail = code & ((1u64 << shift) - 1) as u32;
            let (low, high) = rest.entry(len).or_insert((tail, tail));
            for byte in (0..shift).step_by(8) {
                let value = (tail >> byte) & 0xFF;
                *low = with_byte(*low, byte, value.min((*low >> byte) & 0xFF));
                *high = with_byte(*high, byte, value.max((*high >> byte) & 0xFF));
            }
        }
        let mut runs: Vec<(u8, u32, u32)> = Vec::new();
        for (lead, len) in leads {
            match runs.last_mut() {
                Some((run_len, _, last)) if *run_len == len => *last = lead,
                _ => runs.push((len, lead, lead)),
            }
        }
        let mut spaces: Vec<(u8, u32, u32)> = runs
            .into_iter()
            .map(|(len, first, last)| {
                let shift = 8 * (u32::from(len) - 1);
                let (low_tail, high_tail) = rest[&len];
                (len, first << shift | low_tail, last << shift | high_tail)
            })
            .collect();
        if spaces.is_empty() {
            spaces.push((1, 0x00, 0xFF));
        }
        spaces
    }
}

/// Remove `deleted` codes from each named `/ToUnicode` CMap stream, each by
/// its value at any length the CMap stores it under (see
/// [`UnicodeMap::remove`]), sparing any code `surviving` text still draws
/// with it.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a CMap
/// holding a deleted code cannot be read, or its rewrite, read back, still
/// maps one, so the redaction fails closed rather than leaving a
/// recoverable code->Unicode entry; or as [`UnicodeMap::read`].
pub(crate) fn scrub(
    doc: &mut Document,
    deleted: &BTreeMap<ObjectId, BTreeSet<Vec<u8>>>,
    surviving: &BTreeMap<ObjectId, BTreeSet<Vec<u8>>>,
) -> Result<()> {
    for (&cmap_id, deleted_here) in deleted {
        // Fonts and their CMaps are shared, and a code (the letter `a`)
        // recurs in text that stays: the very codes text still draws stay,
        // while every other entry of a deleted code's value goes.
        let empty = BTreeSet::new();
        let kept = surviving.get(&cmap_id).unwrap_or(&empty);
        let values: BTreeSet<u32> = deleted_here
            .iter()
            .filter_map(|code| key(code))
            .map(|(_, value)| value)
            .collect();
        if values.is_empty() {
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
        if !map.maps(&values, kept) {
            continue;
        }
        map.remove(&values, kept);
        // The CMap written is read back as a viewer would: a deleted code
        // still mapped there fails the redaction.
        let written = map.write();
        let check = Stream::new(Dictionary::new(), written.clone());
        let remains = UnicodeMap::read(doc, &check)?.is_none_or(|read| read.maps(&values, kept));
        if remains {
            return Err(Error::redaction(
                "a deleted glyph's code is still mapped in its rewritten /ToUnicode CMap",
            ));
        }
        if let Ok(Object::Stream(stream)) = doc.get_object_mut(cmap_id) {
            stream.set_plain_content(written);
        }
    }
    Ok(())
}

/// Whether the `len`-byte `code` is one of `values` and not `kept`.
fn removed(len: u8, code: u32, values: &BTreeSet<u32>, kept: &BTreeSet<Vec<u8>>) -> bool {
    values.contains(&code) && !kept.contains(&code_bytes(len, code))
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
/// `step` past its first to: `first` with its last byte stepped, the step
/// staying within that byte, as a range's destinations do (9.10.3).
fn steps(first: &[u16], step: u32, units: &[u16]) -> bool {
    let (Some((&last, head)), Some((&unit, rest))) = (first.split_last(), units.split_last())
    else {
        return false;
    };
    let stepped = u32::from(last) + step;
    head == rest && stepped == u32::from(unit) && stepped >> 8 == u32::from(last) >> 8
}

/// The `len`-byte `code` as its bytes.
fn code_bytes(len: u8, code: u32) -> Vec<u8> {
    code.to_be_bytes()[4 - usize::from(len)..].to_vec()
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
        let map = read(&source()).readable().unwrap();
        assert_eq!(text(&map, 2, 0x20).as_deref(), Some("山"));
        assert_eq!(text(&map, 2, 0x04).as_deref(), Some("fi"));
        assert_eq!(read(&map.write()), map);
    }

    /// A code removed from inside a range leaves the codes around it
    /// mapped as they were.
    #[test]
    fn a_code_is_removed_from_inside_a_range() {
        let mut map = read(&source());
        map.remove(&BTreeSet::from([0x11]), &BTreeSet::new());
        let map = read(&map.write());
        assert_eq!(text(&map, 2, 0x10).as_deref(), Some("a"));
        assert_eq!(text(&map, 2, 0x11), None);
        assert_eq!(text(&map, 2, 0x12).as_deref(), Some("c"));
    }

    /// A one-byte code is removed under whatever length the CMap stores it,
    /// so a viewer reading codes by value alone finds it nowhere; a code
    /// text that stays still draws is spared.
    #[test]
    fn a_code_is_removed_at_every_length_it_is_stored() {
        let mut map = read(&cmap(
            "2 begincodespacerange\n<00> <FF>\n<0000> <FFFF>\nendcodespacerange\n\
            3 beginbfchar\n<0041> <004A>\n<42> <0042>\n<0042> <0062>\nendbfchar",
        ));
        map.remove(&BTreeSet::from([0x41]), &BTreeSet::new());
        map.remove(&BTreeSet::from([0x42]), &BTreeSet::from([vec![0x00, 0x42]]));
        let map = read(&map.write());
        assert_eq!(text(&map, 2, 0x41), None);
        assert_eq!(text(&map, 1, 0x42), None);
        assert_eq!(text(&map, 2, 0x42).as_deref(), Some("b"));
    }

    /// A deleted code that text elsewhere still draws stays, while an entry
    /// of its value at another length, which nothing draws, goes.
    #[test]
    fn a_surviving_code_spares_only_itself() {
        let mut doc = Document::new();
        let cmap = doc.add_object(Stream::new(
            dictionary! {},
            cmap(
                "2 begincodespacerange\n<00> <FF>\n<0000> <FFFF>\nendcodespacerange\n\
                2 beginbfchar\n<41> <0041>\n<0041> <004A>\nendbfchar",
            ),
        ));
        let deleted = BTreeMap::from([(cmap, BTreeSet::from([vec![0x41]]))]);
        scrub(&mut doc, &deleted, &deleted).unwrap();
        let Ok(Object::Stream(stream)) = doc.get_object(cmap) else {
            panic!("cmap");
        };
        let map = read(&stream.content);
        assert_eq!(text(&map, 1, 0x41).as_deref(), Some("A"));
        assert_eq!(text(&map, 2, 0x41), None);
    }

    /// A map whose destinations, written as readable text, outgrow the unit
    /// limit is refused, as reading what is written would be.
    #[test]
    fn a_map_outgrowing_its_limit_when_readable_is_refused() {
        let ligatures: String = "\u{FB01}".repeat(16);
        let destination: String = ligatures
            .encode_utf16()
            .map(|unit| format!("{unit:04X}"))
            .collect();
        let count = MAX_UNITS / 16 - 1;
        let entries: String = (0..count)
            .map(|code| format!("<{code:04X}> <{destination}>\n"))
            .collect();
        let sections: String = entries
            .lines()
            .collect::<Vec<_>>()
            .chunks(100)
            .map(|chunk| {
                format!(
                    "{} beginbfchar\n{}\nendbfchar\n",
                    chunk.len(),
                    chunk.join("\n")
                )
            })
            .collect();
        let map = read(&cmap(&format!(
            "1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n{sections}"
        )));
        let error = map.readable().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
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

    /// A map whose every code was removed is still a CMap lopdf reads.
    #[test]
    fn a_map_of_no_code_is_still_read() {
        let mut map = read(&source());
        let values = map.codes.keys().map(|&(_, code)| code).collect();
        map.remove(&values, &BTreeSet::new());
        assert!(read(&map.write()).codes.is_empty());
    }

    /// Codes of one length span one codespace however far apart they are;
    /// a code of another length between them splits it, so that byte stays
    /// the first byte of the longer code.
    #[test]
    fn the_codespace_keeps_first_bytes_apart() {
        let map = read(&cmap(
            "2 begincodespacerange\n<00> <80>\n<8140> <9FFC>\nendcodespacerange\n\
            4 beginbfchar\n<41> <0041>\n<7A> <007A>\n<8140> <3000>\n<A1> <FF61>\nendbfchar",
        ));
        let written = String::from_utf8(map.write()).unwrap();
        assert!(
            written.contains("3 begincodespacerange\n<41> <7A>\n<8140> <8140>\n<A1> <A1>\n"),
            "{written}"
        );
    }

    /// A range's destinations step within their last byte: codes whose
    /// destinations carry into the byte before are written apart.
    #[test]
    fn a_range_does_not_carry_past_its_last_byte() {
        let map = read(&cmap(
            "1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
            2 beginbfchar\n<01> <00FF>\n<02> <0100>\nendbfchar",
        ));
        let written = String::from_utf8(map.write()).unwrap();
        assert!(written.contains("<01> <00FF>\n<02> <0100>"), "{written}");
        assert_eq!(read(written.as_bytes()), map);
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
