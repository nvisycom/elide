//! [`XlsxStream`]: the body [`Stream<Tabular>`] of a workbook's cells.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{FormatId, Stream};
use elide_core::modality::tabular::{Tabular, TabularLocation, TabularReplacement};
use elide_core::modality::text::TextData;
use elide_core::modality::{Chunk, DataReader, DataWriter, ResolvedHint};
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};

use super::FORMAT_ID;
use super::state::XlsxState;

/// The body [`Stream<Tabular>`] of an XLSX workbook: each cell is independently
/// addressable via a [`TabularLocation`] scoped by sheet name. Redactions land on
/// the shared [`XlsxState`]; the [`XlsxRecombine`](super::recombine::XlsxRecombine)
/// re-packs from it.
pub(super) struct XlsxStream {
    pub(super) state: XlsxState,
    pub(super) cursor: usize,
}

#[async_trait::async_trait]
impl Stream<Tabular> for XlsxStream {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        // The workbook re-pack lives in `XlsxRecombine`, which reads the shared
        // cells; this part's own bytes are ignored by the recombiner.
        Ok(ContentData::new(Bytes::new()))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Tabular>>> {
        let Some(cell) = self.state.cell(self.cursor) else {
            return Ok(None);
        };
        self.cursor += 1;
        let mut location =
            TabularLocation::new(cell.row, cell.column).with_sheet_name(cell.sheet.clone());
        // Attach the column's header text as context, so a context-gated pattern
        // (a payment card wants a nearby `card`/`payment` word) can reach its
        // threshold on a value that carries no such cue on its own. A data cell
        // takes its header from row 0 of the same sheet and column; a header cell
        // is its own context and is not re-hinted with itself.
        let mut hints = Vec::new();
        if cell.row > 0
            && let Some(header) = self.state.column_header(&cell.sheet, cell.column)
        {
            location = location.clone().with_column_name(header.clone());
            let header_location = TabularLocation::new(0, cell.column)
                .with_sheet_name(cell.sheet.clone())
                .with_column_name(header.clone());
            hints.push(ResolvedHint::new(header_location, TextData::new(header)));
        }
        Ok(Some(Chunk {
            location,
            data: TextData::new(cell.text),
            hints,
        }))
    }

    fn lift(&self, chunk: &Chunk<Tabular>, local: TabularLocation) -> Option<TabularLocation> {
        // `local` carries the chunk-local intra-cell range in its offsets; its
        // row/column/sheet are placeholders. Re-anchor onto the chunk's cell. A
        // chunk always names its own sheet, so the lookup is unambiguous; treat
        // an ambiguous or missing match as no source pre-image.
        let cell = self.state.cell_at(&chunk.location).ok().flatten()?;
        let start = local.start_offset.unwrap_or(0);
        let end = local.end_offset.unwrap_or(cell.len());
        if start > end || end > cell.len() {
            return None;
        }
        let mut location =
            TabularLocation::new(chunk.location.row_index, chunk.location.column_index)
                .with_range(start, end);
        if let Some(sheet) = &chunk.location.sheet_name {
            location = location.with_sheet_name(sheet.clone());
        }
        if let Some(name) = &chunk.location.column_name {
            location = location.with_column_name(name.clone());
        }
        Some(location)
    }
}

#[async_trait::async_trait]
impl DataReader<Tabular> for XlsxStream {
    async fn read_at(&self, location: &TabularLocation) -> Result<Option<TextData>> {
        let Some(cell) = self.state.cell_at(location)? else {
            return Ok(None);
        };
        match (location.start_offset, location.end_offset) {
            // A sub-cell range: an unset end means the rest of the cell, matching
            // how a write treats a missing end, so read and write agree.
            (Some(start), end) => {
                let end = end.unwrap_or(cell.len());
                Ok(cell.get(start..end).map(TextData::new))
            }
            // No start: the whole cell.
            (None, _) => Ok(Some(TextData::new(cell))),
        }
    }
}

#[async_trait::async_trait]
impl DataWriter<Tabular> for XlsxStream {
    async fn write_at(&mut self, mut redactions: Redactions<Tabular>) -> Result<()> {
        redactions.sort_by_position();
        // Collect the cell edits, rejecting the whole batch on any unsupported or
        // unresolvable replacement BEFORE mutating any cell — so a failed batch
        // (e.g. a valid cell followed by a `DropRow`) never leaves a partial edit
        // that a later encode would ship.
        let mut edits = Vec::new();
        // Right-to-left so an edit's length delta does not move earlier
        // intra-cell offsets in the same cell.
        for (location, replacement) in redactions.into_iter().rev() {
            match replacement {
                TabularReplacement::Cell(cell) => edits.push((location, cell)),
                // A whole-row or whole-column drop would renumber every `r=`
                // reference across the sheet; that structural rewrite is not yet
                // supported, so refuse it rather than silently keep the data.
                TabularReplacement::DropRow | TabularReplacement::DropColumn => {
                    return Err(Error::new(
                        ErrorKind::CapabilityUnavailable,
                        "XLSX structural row/column drops are not yet supported",
                    ));
                }
            }
        }
        // Applied atomically: every edit is resolved and validated before any
        // cell is mutated, so an unresolvable location fails the whole batch clean.
        self.state.redact_cells(&edits)
    }
}
