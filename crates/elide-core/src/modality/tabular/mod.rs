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
use super::text::{TextData, Token, Tokens};
use super::text_recognizable::TextRecognizable;

/// Tabular modality: cells hold text, so data is [`TextData`] and
/// replacements are [`TabularReplacement`]; only [`TabularLocation`] is
/// tabular-specific.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Tabular;

impl Modality for Tabular {
    type Artifact = Tokens;
    type Data = TextData;
    type Location = TabularLocation;
    type Replacement = TabularReplacement;

    const NAME: &'static str = "tabular";
}

impl TextRecognizable for Tabular {
    fn as_text<'a>(data: &'a TextData, _artifact: Option<&'a Tokens>) -> &'a str {
        data.text.as_str()
    }

    fn locate(
        range: Range<usize>,
        _data: &TextData,
        _artifact: Option<&Tokens>,
    ) -> Option<TabularLocation> {
        // Chunk-local: only the intra-cell byte range is known here; the
        // codec's lift fills the row/column from the chunk.
        Some(TabularLocation::new(0, 0).with_range(range.start, range.end))
    }

    fn as_tokens(artifact: Option<&Tokens>) -> Option<&[Token]> {
        artifact.filter(|t| !t.is_empty()).map(Tokens::as_slice)
    }
}
