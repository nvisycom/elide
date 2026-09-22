//! [`OffsetMap`]: the decoded-text-to-glyph-bytes correspondence for one page.
//!
//! A page's [`text`](super::TextBlock::text) is the decoded logical text a
//! caller runs detection over; each character was drawn by a glyph whose code
//! occupies a byte range of a string operand at some [`Address`]. The map
//! records that correspondence as a list of [`runs`](OffsetRun): each run ties a
//! contiguous stretch of decoded characters to the glyph bytes (at one address)
//! that drew them. A synthetic gap (a word space no glyph drew) is a run with no
//! address. Mapping a detected character range to the exact glyph bytes it must
//! delete is then [`glyph_bytes`](OffsetMap::glyph_bytes).

use super::Address;

/// One stretch of the decoded-to-glyph-bytes correspondence: a contiguous range
/// of decoded characters and the glyph bytes, at one [`Address`], that drew
/// them.
///
/// `chars` is the page-local decoded character range this run covers.
/// `source` is `Some((address, byte_start, byte_end))` for characters a glyph
/// drew, where the byte range is within that address's string operand, or `None`
/// for a synthetic gap (a word space between runs, which no glyph drew and which
/// a redaction leaves alone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetRun {
    /// The page-local decoded character range this run covers.
    pub chars: std::ops::Range<usize>,
    /// The glyph bytes that drew those characters, or `None` for a gap.
    pub source: Option<GlyphBytes>,
}

/// The glyph bytes that drew a run's characters: the string operand address,
/// the byte range within that operand's string, and the font that drew it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphBytes {
    /// Which stream/operation/operand/item the glyph's code lives in.
    pub address: Address,
    /// Start byte offset of the glyph's code within that string operand.
    pub byte_start: usize,
    /// End byte offset (exclusive) of the glyph's code within that operand.
    pub byte_end: usize,
    /// Index into the block's font table of the font that drew the glyph, so its
    /// code can be scrubbed from that font's `/ToUnicode` CMap.
    pub font: u16,
}

/// The decoded-text-to-glyph-bytes correspondence for one page, as a list of
/// [`runs`](OffsetRun) contiguous in the decoded-character dimension.
///
/// [`glyph_bytes`](Self::glyph_bytes) maps a detected character range to the
/// [`GlyphBytes`] it must delete, from which the redactor groups byte ranges by
/// [`Address`] and collects the glyph codes to scrub from `/ToUnicode`. A glyph
/// whose one code drew several characters (a ligature) appears in several runs
/// sharing the same [`GlyphBytes`], so callers coalesce by address + byte range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OffsetMap {
    runs: Vec<OffsetRun>,
}

impl OffsetMap {
    /// Build a map from an explicit list of runs.
    #[must_use]
    pub fn new(runs: Vec<OffsetRun>) -> Self {
        Self { runs }
    }

    /// The map's runs, in decoded-character order.
    #[must_use]
    pub fn runs(&self) -> &[OffsetRun] {
        &self.runs
    }

    /// The glyph bytes drawing the characters in `chars`, in decoded order.
    ///
    /// Yields one [`GlyphBytes`] per glyph the range covers; gaps contribute
    /// nothing. A ligature glyph spanning several characters yields its shared
    /// [`GlyphBytes`] once per covered character, callers coalesce by address +
    /// byte range (a byte span drained twice would corrupt the string).
    pub fn glyph_bytes(
        &self,
        chars: std::ops::Range<usize>,
    ) -> impl Iterator<Item = GlyphBytes> + '_ {
        self.runs
            .iter()
            .filter(move |run| run.chars.start < chars.end && run.chars.end > chars.start)
            .filter_map(|run| run.source)
    }
}
