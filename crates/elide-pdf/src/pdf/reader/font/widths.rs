//! Glyph advance widths, read from a font dictionary, for text-space geometry.
//!
//! Reconstructing where a glyph lands needs its advance width — how far the pen
//! moves after drawing it. A simple font carries a `/Widths` array indexed by
//! byte code from `/FirstChar`; a composite (Type0) font carries a `/W` array
//! keyed by CID with a `/DW` default. A simple font that names one of the
//! standard 14 fonts and embeds no program may give no `/Widths`; a reader
//! then supplies the face's own (see [`Standard`]). This module reads each into
//! a uniform [`WidthSource`] that answers [`advance`](WidthSource::advance) in
//! text-space thousandths per unit of font size (the unit `/Widths` and `/W`
//! use, `1000` to the em). A Type3 font's `/Widths` are in its glyph space, so
//! are scaled by its `/FontMatrix` into that unit.
//!
//! Widths place glyphs: they decide where a word space is read, and the boxes a
//! detection's rectangles and a reviewer's mark are resolved against. Deletion
//! itself works off glyph byte ranges, so a glyph whose width is unknown still
//! deletes. Every lookup is best-effort: a missing or unparseable table yields
//! [`WidthSource::Unknown`], and the caller falls back to treating any
//! positional move as a word boundary rather than failing.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object};

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
    /// exhaust memory. Lookups search the (few) ranges instead.
    Composite {
        dw: f64,
        w: BTreeMap<u32, f64>,
        ranges: Vec<CidRange>,
    },
    /// A standard font with no `/Widths`: each code's width in the face, `None`
    /// for a code whose glyph the face gives no width.
    Standard(Box<[Option<f64>; 256]>),
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
                let idx = i64::from(code) - first_char;
                let within = usize::try_from(idx)
                    .ok()
                    .and_then(|i| widths.get(i))
                    .copied();
                Some(within.unwrap_or(*missing))
            }
            WidthSource::Composite { dw, w, ranges } => {
                // Explicit per-CID width first, then a covering range, then the
                // default. Ranges are few (one `/W` range entry each), so a linear
                // scan is cheap and avoids expanding them per CID.
                if let Some(width) = w.get(&code) {
                    return Some(*width);
                }
                let in_range = ranges
                    .iter()
                    .find(|r| code >= r.first && code <= r.last)
                    .map(|r| r.width);
                Some(in_range.unwrap_or(*dw))
            }
            WidthSource::Standard(widths) => usize::try_from(code)
                .ok()
                .and_then(|code| *widths.get(code)?),
            WidthSource::Unknown => None,
        }
    }

    /// The widths of a simple font: its `/Widths` from `/FirstChar`, with
    /// `/FontDescriptor` `/MissingWidth` as the out-of-range fallback, scaled
    /// by its `/FontMatrix` for a Type3 font; else, for a standard font, the
    /// face's own.
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
        Standard::of(doc, font)
            .and_then(|standard| standard.widths(doc, font))
            .map_or(WidthSource::Unknown, |widths| {
                WidthSource::Standard(Box::new(widths))
            })
    }

    /// `/Widths` + `/FirstChar` of a simple font, each width times `scale`.
    fn from_widths(doc: &Document, font: &Dictionary, scale: f64) -> Option<WidthSource> {
        let first_char = font.get(b"FirstChar").ok()?.as_i64().ok()?;
        let widths = doc
            .resolve_array(font.get(b"Widths").ok()?)?
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
        // `/DescendantFonts` is a one-element array holding the CIDFont (inline or
        // by reference).
        let Some(cid_font) = font
            .get(b"DescendantFonts")
            .ok()
            .and_then(|o| doc.resolve_array(o))
            .and_then(|arr| arr.first())
            .and_then(|o| doc.resolve_dict(o))
        else {
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
        let em = |key: &[u8]| {
            descriptor?
                .get(key)
                .ok()
                .map(|o| o.as_number().unwrap_or(0.0))
                .filter(|v| *v != 0.0)
                .map(|v| v * unit)
        };
        let face = |value: fn(Standard) -> Option<f64>| value(standard?).map(|v| v / 1000.0);
        // Top: a cap height (tight to the drawn glyphs) if either gives one,
        // else an ascent.
        let top = em(b"CapHeight")
            .or_else(|| face(Standard::cap_height))
            .or_else(|| em(b"Ascent"))
            .or_else(|| face(Standard::ascender))
            .unwrap_or(default.top);
        // Bottom sits *below* the baseline, so it must be negative. The spec says
        // `/Descent` is typically negative, but some producers (e.g. Skia) store
        // it as a positive magnitude; normalize to `-|value|` so either
        // convention yields a below-baseline extent (otherwise the glyph box is
        // placed entirely above the baseline and comes out too short).
        let bottom = em(b"Descent")
            .or_else(|| face(Standard::descender))
            .map(|d| -d.abs())
            .unwrap_or(default.bottom);
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
        font.get(b"DescendantFonts")
            .ok()
            .and_then(|o| doc.resolve_array(o))
            .and_then(|arr| arr.first())
            .and_then(|o| doc.resolve_dict(o))?
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
/// The array mixes two forms (ISO 32000-1 §9.7.4.3): `c [w1 w2 …]` assigns
/// consecutive CIDs starting at `c` (expanded into `out`, bounded by the list
/// length the document actually wrote), and `c_first c_last w` assigns one width
/// to every CID in the inclusive range (kept as a [`CidRange`] rather than
/// expanded — the span is untrusted and may be enormous). A malformed tail is
/// ignored; widths are best-effort.
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
                    if let Ok(cid) = u32::try_from(start + offset as i64) {
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
        assert_eq!(face.advance(256), None);
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
}
