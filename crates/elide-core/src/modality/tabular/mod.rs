//! [`Tabular`] modality: spreadsheet and CSV content addressed by cell.
//!
//! A tabular cell holds text and is redacted like text, so this modality
//! reuses [`TextData`] as its payload and [`TextReplacement`] as its
//! treatment. Only the *location* is tabular: a sheet, a row, a column,
//! and an optional byte range within the cell for sub-cell entities.

mod location;

use std::ops::Range;

pub use self::location::TabularLocation;
use super::Modality;
use super::text::{TextData, TextReplacement};
use super::text_backed::{TextBacked, TextRecognizable};
use crate::recognition::RecognizerContext;

/// Tabular modality: cells hold text, so data is [`TextData`] and
/// replacements are [`TextReplacement`]; only [`TabularLocation`] is
/// tabular-specific.
#[derive(Debug, Clone, Copy)]
pub struct Tabular;

impl Modality for Tabular {
    type Data = TextData;
    type Location = TabularLocation;
    type Replacement = TextReplacement;

    const NAME: &'static str = "tabular";
}

impl TextRecognizable for Tabular {
    fn as_text<'a>(data: &'a TextData, _ctx: &'a RecognizerContext<'_, Self>) -> &'a str {
        data.text.as_str()
    }

    fn locate(
        range: Range<usize>,
        _data: &TextData,
        _ctx: &RecognizerContext<'_, Self>,
    ) -> TabularLocation {
        // Chunk-local: only the intra-cell byte range is known here; the
        // codec's lift fills the row/column from the chunk.
        TabularLocation::new(0, 0).with_range(range.start, range.end)
    }
}

impl TextBacked for Tabular {
    fn span(location: &TabularLocation) -> Range<usize> {
        location.start_offset.unwrap_or(0)..location.end_offset.unwrap_or(0)
    }
}
