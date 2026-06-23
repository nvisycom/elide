//! [`Tabular`] modality: spreadsheet and CSV content addressed by cell.
//!
//! A tabular cell holds text and is recognized like text, so this modality
//! reuses [`TextData`] as its payload. Redaction is a [`TabularReplacement`]:
//! usually a text treatment applied to the cell, but also structural drops
//! (a whole row or column) that the text model can't express. The *location*
//! is tabular: a sheet, a row, a column, and an optional byte range within
//! the cell for sub-cell entities.

mod location;
mod replacement;

use std::ops::Range;

pub use self::location::TabularLocation;
pub use self::replacement::TabularReplacement;
use super::Modality;
use super::text::TextData;
use super::text_recognizable::TextRecognizable;
use crate::recognition::Artifacts;

/// Tabular modality: cells hold text, so data is [`TextData`] and
/// replacements are [`TabularReplacement`]; only [`TabularLocation`] is
/// tabular-specific.
#[derive(Debug, Clone, Copy)]
pub struct Tabular;

impl Modality for Tabular {
    type Data = TextData;
    type Location = TabularLocation;
    type Replacement = TabularReplacement;

    const NAME: &'static str = "tabular";
}

impl TextRecognizable for Tabular {
    fn as_text<'a>(data: &'a TextData, _artifacts: &'a Artifacts) -> &'a str {
        data.text.as_str()
    }

    fn locate(
        range: Range<usize>,
        _data: &TextData,
        _artifacts: &Artifacts,
    ) -> Option<TabularLocation> {
        // Chunk-local: only the intra-cell byte range is known here; the
        // codec's lift fills the row/column from the chunk.
        Some(TabularLocation::new(0, 0).with_range(range.start, range.end))
    }
}
