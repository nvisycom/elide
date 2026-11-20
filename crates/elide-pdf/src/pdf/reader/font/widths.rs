//! Glyph advance widths, read from a font dictionary, for text-space geometry.
//!
//! Reconstructing where a glyph lands needs its advance width — how far the pen
//! moves after drawing it. A simple font carries a `/Widths` array indexed by
//! byte code from `/FirstChar`; a composite (Type0) font carries a `/W` array
//! keyed by CID with a `/DW` default. A simple font that names one of the
//! standard 14 fonts and embeds no program may give no `/Widths`; a reader
//! then supplies the face's own (see [`Standard`]). A simple font that embeds
//! its program and gives none takes the program's (see [`embedded_widths`]).
//! This module reads each into a uniform [`WidthSource`] that answers
//! [`advance`](WidthSource::advance) in text-space thousandths per unit of
//! font size (the unit `/Widths` and `/W` use, `1000` to the em). A Type3 font's `/Widths` are in its glyph space, so
//! are scaled by its `/FontMatrix` into that unit.
//!
//! Widths place glyphs: they decide where a word space is read, and the boxes a
//! detection's rectangles and a reviewer's mark are resolved against. Deletion
//! itself works off glyph byte ranges, so a glyph whose width is unknown still
//! deletes. Every lookup is best-effort: a missing or unparseable table yields
//! [`WidthSource::Unknown`], and the caller falls back to treating any
//! positional move as a word boundary rather than failing.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;

use lopdf::{Dictionary, Document, Object};

use super::descendant;
use super::program::embedded_widths;
use super::standard::Standard;
use crate::pdf::reader::{DictionaryExt, DocumentExt, ObjectExt};

/// The default glyph width (glyph-space thousandths) when a font declares none:
/// the PDF default for a composite font's `/DW`, and a reasonable stand-in for a
/// simple font's `/MissingWidth`.
const DEFAULT_WIDTH: f64 = 1000.0;

/// A font's glyph advance widths, in glyph-space thousandths.
#[derive(Debug, Clone)]
pub(crate) enum WidthSource {
    /// A simple font: `widths[code - first_char]`, falling back to `missing` for a
    /// code outside the array.
    Simple {
        first_char: i64,
        widths: Vec<f64>,
        missing: f64,
    },
    /// A composite (Type0) font: a per-CID width `w`, then a covering `range`,
    /// then the default `dw`. For an `Identity-H`/`Identity-V` encoding the
    /// character code equals the CID, so the caller keys directly by the decoded
    /// code.
    ///
    /// Ranges are kept compact rather than expanded into `w`: a `/W` entry of the
    /// form `c_first c_last w` may name an enormous CID span (it is document-
    /// supplied, untrusted), so expanding it per CID would let a hostile font
    /// exhaust memory. They are held disjoint and sorted, so a lookup is a
    /// binary search however many the font gives.
    Composite {
        dw: f64,
        w: BTreeMap<u32, f64>,
        ranges: Vec<CidRange>,
    },
    /// No usable width table: the caller treats positional moves as word
    /// boundaries instead of computing an exact advance.
    Unknown,
}

impl WidthSource {
    /// Read the width table from a font dictionary, best-effort.
    ///
    /// A `/Type0` font's widths live on its descendant CIDFont; any other font is
    /// treated as simple (`/Widths` + `/FirstChar`). Anything missing or
    /// unreadable yields [`Unknown`](WidthSource::Unknown).
    pub(crate) fn from_font(doc: &Document, font: &Dictionary) -> WidthSource {
        if font.is_subtype(doc, b"Type0") {
            Self::from_type0(doc, font)
        } else {
            Self::from_simple(doc, font)
        }
    }

    /// The advance width of `code` in glyph-space thousandths, or `None` when the
    /// table cannot answer (so the caller falls back to gap-presence).
    pub(crate) fn advance(&self, code: u32) -> Option<f64> {
        match self {
            WidthSource::Simple {
                first_char,
                widths,
                missing,
            } => {
                let within = i64::from(code)
                    .checked_sub(*first_char)
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| widths.get(index))
                    .copied();
                Some(within.unwrap_or(*missing))
            }
            WidthSource::Composite { dw, w, ranges } => {
                // Explicit per-CID width first, then a covering range, then the
                // default.
                if let Some(width) = w.get(&code) {
                    return Some(*width);
                }
                let in_range = ranges
                    .get(ranges.partition_point(|range| range.last < code))
                    .filter(|range| range.first <= code)
                    .map(|range| range.width);
                Some(in_range.unwrap_or(*dw))
            }
            WidthSource::Unknown => None,
        }
    }

    /// The widths of a simple font: its `/Widths` from `/FirstChar`, with
    /// `/FontDescriptor` `/MissingWidth` as the out-of-range fallback, scaled
    /// by its `/FontMatrix` for a Type3 font; else, for a standard font, the
    /// face's own; else those of the program it embeds.
    fn from_simple(doc: &Document, font: &Dictionary) -> WidthSource {
        let scale = if font.is_subtype(doc, b"Type3") {
            match type3_scale(doc, font) {
                Some(scale) => scale.0 * 1000.0,
                None => return WidthSource::Unknown,
            }
        } else {
            1.0
        };
        if let Some(widths) = Self::from_widths(doc, font, scale) {
            return widths;
        }
        supplied_widths(doc, font).map_or(WidthSource::Unknown, |widths| WidthSource::Simple {
            first_char: 0,
            widths: widths.to_vec(),
            missing: 0.0,
        })
    }

    /// `/Widths` + `/FirstChar` of a simple font, each width times `scale`.
    fn from_widths(doc: &Document, font: &Dictionary, scale: f64) -> Option<WidthSource> {
        let (first_char, widths) = own_widths(doc, font)?;
        let widths = widths
            .iter()
            .map(|o| o.as_number().unwrap_or(0.0) * scale)
            .collect();
        let missing = font
            .get(b"FontDescriptor")
            .ok()
            .and_then(|o| doc.resolve_dict(o))
            .and_then(|fd| fd.get(b"MissingWidth").ok())
            .map(|o| o.as_number().unwrap_or(0.0) * scale)
            .unwrap_or(0.0);
        Some(WidthSource::Simple {
            first_char,
            widths,
            missing,
        })
    }

    /// `/W` + `/DW` of a Type0 font's descendant CIDFont.
    fn from_type0(doc: &Document, font: &Dictionary) -> WidthSource {
        let Some(cid_font) = descendant(doc, font) else {
            return WidthSource::Unknown;
        };

        let dw = cid_font
            .get(b"DW")
            .ok()
            .map(|o| o.as_number().unwrap_or(0.0))
            .unwrap_or(DEFAULT_WIDTH);

        let mut w = BTreeMap::new();
        let mut ranges = Vec::new();
        if let Some(entries) = cid_font.get(b"W").ok().and_then(|o| doc.resolve_array(o)) {
            parse_w_array(entries, &mut w, &mut ranges);
        }
        WidthSource::Composite { dw, w, ranges }
    }
}

/// The `/FirstChar` and `/Widths` the simple `font` gives, when it gives
/// both.
fn own_widths<'a>(doc: &'a Document, font: &'a Dictionary) -> Option<(i64, &'a [Object])> {
    let first_char = font.get(b"FirstChar").ok()?.as_i64().ok()?;
    let widths = doc.resolve_array(font.get(b"Widths").ok()?)?;
    Some((first_char, widths))
}

/// The width of each code of the simple `font`, in thousandths of an em,
/// when it gives none of its own and a reader supplies them: a standard
/// face's (see [`Standard`]), or those of the program it embeds (see
/// [`embedded_widths`]). The rebuild writes them into the font, so these are
/// the widths every viewer then places its glyphs by.
pub(crate) fn supplied_widths(doc: &Document, font: &Dictionary) -> Option<[f64; 256]> {
    if own_widths(doc, font).is_some() {
        return None;
    }
    match Standard::of(doc, font) {
        Some(standard) => Some(standard.widths(doc, font)),
        None => embedded_widths(doc, font),
    }
}

/// One `c_first c_last w` entry of a `/W` array, kept as a range rather than
/// expanded per CID.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CidRange {
    first: u32,
    last: u32,
    width: f64,
}

/// A font's vertical glyph extent, in text-space em fractions (glyph-space
/// thousandths / 1000), for building a glyph's box height.
///
/// Read from the `/FontDescriptor` (a Type0 font's descriptor lives on its
/// descendant CIDFont). The box top is the `/CapHeight` — the top of the drawn
/// glyphs, which matches a viewer's tight text box better than the full
/// `/Ascent` (which reserves room above capitals the text does not use); it
/// falls back to `/Ascent` when a font declares no cap height. The bottom is the
/// `/Descent`. A standard font without them takes its face's (a cap height
/// from either ahead of an ascent from either), and anything
/// still missing falls back to a default em box rather than failing: metrics
/// feed only the box height, never redaction. A Type3 font's are in its glyph
/// space, so are scaled by its `/FontMatrix`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GlyphMetrics {
    /// Top of the box above the baseline, as an em fraction (typically ~0.66
    /// from cap height, ~0.75 from ascent).
    pub(crate) top: f64,
    /// Bottom of the box below the baseline, as a (negative) em fraction
    /// (typically ~-0.2).
    pub(crate) bottom: f64,
}

impl Default for GlyphMetrics {
    fn default() -> Self {
        // A plausible Latin em box when a font declares no metrics.
        GlyphMetrics {
            top: 0.7,
            bottom: -0.2,
        }
    }
}

impl GlyphMetrics {
    /// Read the glyph box extent from a font dictionary, best-effort (defaults on
    /// any missing piece). A `/Type0` font's descriptor is on its descendant
    /// CIDFont.
    pub(crate) fn from_font(doc: &Document, font: &Dictionary) -> GlyphMetrics {
        let default = GlyphMetrics::default();
        let unit = if font.is_subtype(doc, b"Type3") {
            match type3_scale(doc, font) {
                Some((_, vertical)) => vertical,
                None => return default,
            }
        } else {
            0.001
        };
        let descriptor = font_descriptor(doc, font);
        let standard = Standard::of(doc, font);
        // Each value with the scale into an em fraction: the descriptor's in
        // glyph space, the standard face's in thousandths.
        let em = |key: &[u8]| {
            descriptor?
                .get(key)
                .ok()
                .map(|o| o.as_number().unwrap_or(0.0))
                .filter(|v| *v != 0.0)
                .map(|v| (v, unit))
        };
        let face = |value: fn(Standard) -> Option<f64>| value(standard?).map(|v| (v, 0.001));
        // A Type3 matrix with a negative vertical scale draws glyphs upside
        // down, so the default box is flipped with them.
        let flip = unit.signum();
        // Top: a cap height (tight to the drawn glyphs) if either gives one,
        // else an ascent.
        let top = em(b"CapHeight")
            .or_else(|| face(Standard::cap_height))
            .or_else(|| em(b"Ascent"))
            .or_else(|| face(Standard::ascender))
            .map_or(default.top * flip, |(v, scale)| v * scale);
        // Bottom sits *below* the baseline in glyph space, so it must be
        // negative there. The spec says `/Descent` is typically negative, but
        // some producers (e.g. Skia) store it as a positive magnitude;
        // normalize to `-|value|` before scaling so either convention yields a
        // below-baseline extent.
        let bottom = em(b"Descent")
            .or_else(|| face(Standard::descender))
            .map_or(default.bottom * flip, |(v, scale)| -v.abs() * scale);
        // Under a flipped matrix the two ends swap sides of the baseline.
        let (top, bottom) = (top.max(bottom), top.min(bottom));
        GlyphMetrics { top, bottom }
    }
}

/// The horizontal and vertical scale of the Type3 `font`'s `/FontMatrix`,
/// from glyph space to text space; `None` when it has none readable, or one
/// that turns a glyph's advance off the baseline.
fn type3_scale(doc: &Document, font: &Dictionary) -> Option<(f64, f64)> {
    let [a, b, _, d, _, _] = doc.resolve_numbers::<6>(font.get(b"FontMatrix").ok()?)?;
    (b == 0.0 && a.is_finite() && d.is_finite()).then_some((a, d))
}

/// Resolve a font's `/FontDescriptor`, following a `/Type0` font to its
/// descendant CIDFont (whose descriptor carries the metrics).
fn font_descriptor<'a>(doc: &'a Document, font: &'a Dictionary) -> Option<&'a Dictionary> {
    let owner = if font.is_subtype(doc, b"Type0") {
        descendant(doc, font)?
    } else {
        font
    };
    owner
        .get(b"FontDescriptor")
        .ok()
        .and_then(|o| doc.resolve_dict(o))
}

/// Parse a Type0 `/W` array into explicit per-CID widths plus compact ranges.
///
/// The array mixes two forms (9.7.4.3): `c [w1 w2 …]` assigns consecutive
/// CIDs starting at `c` (expanded into `out`, bounded by the list length the
/// document actually wrote), and `c_first c_last w` assigns one width to every
/// CID in the inclusive range (kept as a [`CidRange`] rather than expanded —
/// the span is untrusted and may be enormous). Where ranges overlap, the
/// first gives the width, as PDFium reads them; they are kept
/// [`disjoint`]. A malformed tail is ignored; widths are best-effort.
fn parse_w_array(entries: &[Object], out: &mut BTreeMap<u32, f64>, ranges: &mut Vec<CidRange>) {
    let mut i = 0;
    while i < entries.len() {
        let Some(start) = entries.get(i).and_then(|o| o.as_i64().ok()) else {
            break;
        };
        match entries.get(i + 1) {
            // `c [w1 w2 …]`: consecutive CIDs from `start`. Safe to expand — the
            // count is the length of the array the document itself supplied.
            Some(Object::Array(list)) => {
                for (offset, width) in list.iter().enumerate() {
                    let cid = i64::try_from(offset)
                        .ok()
                        .and_then(|offset| start.checked_add(offset))
                        .and_then(|cid| u32::try_from(cid).ok());
                    if let Some(cid) = cid {
                        out.insert(cid, width.as_number().unwrap_or(0.0));
                    }
                }
                i += 2;
            }
            // `c_first c_last w`: one width across the inclusive CID range. Kept
            // compact, never expanded, so a huge `last` cannot exhaust memory.
            Some(end_obj) => {
                let Some(end) = end_obj.as_i64().ok() else {
                    break;
                };
                let Some(width) = entries.get(i + 2).map(|o| o.as_number().unwrap_or(0.0)) else {
                    break;
                };
                if let (Ok(first), Ok(last)) = (u32::try_from(start), u32::try_from(end))
                    && first <= last
                {
                    ranges.push(CidRange { first, last, width });
                }
                i += 3;
            }
            None => break,
        }
    }
    *ranges = disjoint(mem::take(ranges));
}

/// `ranges`, in the order a `/W` array gives them, as disjoint ranges sorted
/// by CID, each CID the width of the first range covering it.
///
/// A sweep over the ranges' ends keeps the ranges covering each span between
/// two ends by their order, the first of them giving its width, so ranges
/// that overlap any number of others are split in `O(n log n)`.
fn disjoint(ranges: Vec<CidRange>) -> Vec<CidRange> {
    let mut starts: Vec<(u32, usize)> = ranges
        .iter()
        .enumerate()
        .map(|(order, range)| (range.first, order))
        .collect();
    starts.sort_unstable();
    let mut ends: Vec<u64> = ranges
        .iter()
        .flat_map(|range| [u64::from(range.first), u64::from(range.last) + 1])
        .collect();
    ends.sort_unstable();
    ends.dedup();
    let mut covering = BTreeSet::new();
    let mut started = starts.iter().peekable();
    let mut out: Vec<CidRange> = Vec::new();
    for span in ends.windows(2) {
        let (first, past) = (span[0], span[1]);
        while let Some(&&(start, order)) = started.peek()
            && u64::from(start) <= first
        {
            covering.insert(order);
            started.next();
        }
        // A range that ended before the span is dropped once it is the first.
        while let Some(&order) = covering.first()
            && u64::from(ranges[order].last) < first
        {
            covering.pop_first();
        }
        let Some(&order) = covering.first() else {
            continue;
        };
        let (Ok(first), Ok(last)) = (u32::try_from(first), u32::try_from(past - 1)) else {
            continue;
        };
        let width = ranges[order].width;
        match out.last_mut() {
            Some(previous)
                if previous.last.checked_add(1) == Some(first) && previous.width == width =>
            {
                previous.last = last;
            }
            _ => out.push(CidRange { first, last, width }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    fn type3(matrix: [f64; 6]) -> Dictionary {
        dictionary! {
            "Type" => "Font",
            "Subtype" => "Type3",
            "FontMatrix" => matrix.iter().map(|&v| Object::Real(v as f32)).collect::<Vec<_>>(),
            "FirstChar" => 65,
            "Widths" => vec![50.into(), 80.into()],
            "FontDescriptor" => dictionary! { "CapHeight" => 70, "Descent" => -20 },
        }
    }

    /// A Type3 font's widths and metrics are in its glyph space, scaled into
    /// text space by its `/FontMatrix`.
    #[test]
    fn type3_widths_follow_the_font_matrix() {
        let doc = Document::with_version("1.4");
        let font = type3([0.01, 0.0, 0.0, 0.01, 0.0, 0.0]);
        let widths = WidthSource::from_font(&doc, &font);
        assert_eq!(widths.advance(65).map(f64::round), Some(500.0));
        assert_eq!(widths.advance(66).map(f64::round), Some(800.0));
        let metrics = GlyphMetrics::from_font(&doc, &font);
        assert!((metrics.top - 0.7).abs() < 1e-6, "{metrics:?}");
        assert!((metrics.bottom + 0.2).abs() < 1e-6, "{metrics:?}");
    }

    /// A Type3 font whose matrix flips glyphs vertically keeps its full box,
    /// above the baseline as well as below.
    #[test]
    fn a_flipped_type3_font_keeps_its_box() {
        let doc = Document::with_version("1.4");
        let font = type3([0.01, 0.0, 0.0, -0.01, 0.0, 0.0]);
        let metrics = GlyphMetrics::from_font(&doc, &font);
        assert!((metrics.top - 0.2).abs() < 1e-6, "{metrics:?}");
        assert!((metrics.bottom + 0.7).abs() < 1e-6, "{metrics:?}");
    }

    /// A Type3 font whose matrix turns its advance off the baseline, or that
    /// has none, has no widths a reader can place glyphs by.
    #[test]
    fn a_type3_font_without_a_usable_matrix_has_unknown_widths() {
        let doc = Document::with_version("1.4");
        let rotated = type3([0.0, 0.01, -0.01, 0.0, 0.0, 0.0]);
        assert!(WidthSource::from_font(&doc, &rotated).advance(65).is_none());
        let mut missing = type3([0.01, 0.0, 0.0, 0.01, 0.0, 0.0]);
        missing.remove(b"FontMatrix");
        assert!(WidthSource::from_font(&doc, &missing).advance(65).is_none());
    }

    /// A standard font's own `/Widths` win over its face's; without them it
    /// takes the face's widths and vertical extent.
    #[test]
    fn a_standard_font_falls_back_to_its_face() {
        let doc = Document::with_version("1.4");
        let mut font = dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Times-Roman",
        };
        let face = WidthSource::from_font(&doc, &font);
        assert_eq!(face.advance(u32::from(b'A')), Some(722.0));
        assert_eq!(face.advance(0x80), Some(0.0), "no glyph");
        let metrics = GlyphMetrics::from_font(&doc, &font);
        assert!((metrics.top - 0.662).abs() < 1e-6, "{metrics:?}");
        assert!((metrics.bottom + 0.217).abs() < 1e-6, "{metrics:?}");

        font.set("FirstChar", 65);
        font.set("Widths", vec![Object::Integer(100)]);
        let own = WidthSource::from_font(&doc, &font);
        assert_eq!(own.advance(u32::from(b'A')), Some(100.0));
    }

    fn int(n: i64) -> Object {
        Object::Integer(n)
    }

    /// The `c [w …]` form expands per CID; the `c_first c_last w` form is kept as
    /// a compact range and resolved at lookup — both answer `advance` correctly.
    #[test]
    fn w_array_resolves_individual_and_range_forms() {
        // `[ 10 [ 100 200 ] 500 1000 750 ]`: CIDs 10,11 explicit; 500..=1000 ranged.
        let entries = vec![
            int(10),
            Object::Array(vec![int(100), int(200)]),
            int(500),
            int(1000),
            int(750),
        ];
        let mut w = BTreeMap::new();
        let mut ranges = Vec::new();
        parse_w_array(&entries, &mut w, &mut ranges);
        let src = WidthSource::Composite {
            dw: 1000.0,
            w,
            ranges,
        };
        assert_eq!(src.advance(10), Some(100.0));
        assert_eq!(src.advance(11), Some(200.0));
        assert_eq!(src.advance(700), Some(750.0), "inside the range");
        assert_eq!(src.advance(1000), Some(750.0), "range is inclusive");
        assert_eq!(src.advance(42), Some(1000.0), "falls back to DW");
    }

    /// A hostile `/W` range spanning the whole CID space is stored compactly, not
    /// expanded — parsing is instant and uses no per-CID memory.
    #[test]
    fn a_huge_w_range_is_not_expanded() {
        let entries = vec![int(0), int(u32::MAX as i64), int(500)];
        let mut w = BTreeMap::new();
        let mut ranges = Vec::new();
        parse_w_array(&entries, &mut w, &mut ranges);
        assert!(w.is_empty(), "no per-CID expansion");
        assert_eq!(ranges.len(), 1, "one compact range");
        let src = WidthSource::Composite {
            dw: 1000.0,
            w,
            ranges,
        };
        assert_eq!(src.advance(3_000_000_000), Some(500.0));
    }

    /// The composite widths a `/W` array of `entries` gives, by default 1000.
    fn composite(entries: Vec<Object>) -> WidthSource {
        let mut w = BTreeMap::new();
        let mut ranges = Vec::new();
        parse_w_array(&entries, &mut w, &mut ranges);
        WidthSource::Composite {
            dw: 1000.0,
            w,
            ranges,
        }
    }

    /// Where `/W` ranges overlap, the first gives the width, as PDFium reads
    /// them; a CID none covers takes the default.
    #[test]
    fn the_first_of_overlapping_ranges_gives_the_width() {
        let widths = composite(vec![
            10.into(),
            20.into(),
            100.into(),
            0.into(),
            30.into(),
            200.into(),
            15.into(),
            40.into(),
            300.into(),
        ]);
        assert_eq!(widths.advance(5), Some(200.0));
        assert_eq!(widths.advance(10), Some(100.0));
        assert_eq!(widths.advance(20), Some(100.0));
        assert_eq!(widths.advance(25), Some(200.0));
        assert_eq!(widths.advance(35), Some(300.0));
        assert_eq!(widths.advance(41), Some(1000.0));
    }

    /// Many overlapping ranges are split once, not searched for each glyph,
    /// and widths at the ends of the integer range don't overflow.
    #[test]
    fn hostile_widths_are_bounded() {
        let entries: Vec<Object> = (0..100_000_i64)
            .flat_map(|i| [i.into(), (i + 100_000).into(), 500.into()])
            .collect();
        let widths = composite(entries);
        let WidthSource::Composite { ranges, .. } = &widths else {
            panic!("composite");
        };
        assert!(ranges.len() <= 200_001);
        assert_eq!(widths.advance(150_000), Some(500.0));

        let extreme = composite(vec![
            i64::MAX.into(),
            vec![Object::from(1), 2.into()].into(),
        ]);
        assert_eq!(extreme.advance(0), Some(1000.0));
        let simple = WidthSource::Simple {
            first_char: i64::MIN,
            widths: vec![1.0],
            missing: 7.0,
        };
        assert_eq!(simple.advance(65), Some(7.0));
    }
}
