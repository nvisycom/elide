//! CSV handler: holds parsed rows and streams them cell by cell, with
//! intra-cell random-access reads and redactions.
//!
//! Cells hold text, so the handler reuses [`TextData`] as the chunk
//! payload and [`TextReplacement`] as the replacement, applying edits
//! through the shared text-redaction helper. Only the *location* is
//! tabular: a `(row, column)` address plus an optional intra-cell byte
//! range.

use elide_core::modality::tabular::{Tabular, TabularLocation};
use elide_core::modality::text::{TextData, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};

use crate::content::ContentData;
use crate::handler::redact;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the CSV codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.tabular.csv");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Tabular, _>(FORMAT_ID.clone(), super::csv_loader::CsvLoader::default())
        .with_extensions(["csv"])
        .with_content_types(["text/csv"])
}

/// Parsed CSV content: an optional header row plus the data rows, with
/// the metadata needed to re-serialize byte-for-byte.
#[derive(Debug, Clone)]
pub(crate) struct CsvData {
    /// Header row, when the source had one.
    pub headers: Option<Vec<String>>,
    /// Data rows (excluding the header).
    pub rows: Vec<Vec<String>>,
    /// Field delimiter byte.
    pub delimiter: u8,
    /// Whether the original ended with a newline.
    pub trailing_newline: bool,
}

/// Streaming cursor over cells, row-major.
#[derive(Debug, Default)]
struct Cursor {
    row: u32,
    col: u32,
}

/// Handler for parsed CSV content. Each cell is independently addressable
/// via a [`TabularLocation`].
#[derive(Debug)]
pub(crate) struct CsvHandler {
    data: CsvData,
    cursor: Cursor,
}

impl CsvHandler {
    /// Wrap parsed CSV data; the streaming cursor starts at the top-left.
    pub(crate) fn new(data: CsvData) -> Self {
        Self {
            data,
            cursor: Cursor::default(),
        }
    }

    /// Total number of addressable rows, header included.
    fn total_rows(&self) -> u32 {
        let header = u32::from(self.data.headers.is_some());
        header + self.data.rows.len() as u32
    }

    /// Number of cells in `row`, or `None` when the row is out of range.
    fn row_len(&self, row: u32) -> Option<usize> {
        self.row_cells(row).map(Vec::len)
    }

    /// The cells of `row` (header row 0 when present, then data rows).
    fn row_cells(&self, row: u32) -> Option<&Vec<String>> {
        match &self.data.headers {
            Some(headers) if row == 0 => Some(headers),
            Some(_) => self.data.rows.get((row - 1) as usize),
            None => self.data.rows.get(row as usize),
        }
    }

    /// Mutable cells of `row`, with the same addressing as [`row_cells`].
    ///
    /// [`row_cells`]: Self::row_cells
    fn row_cells_mut(&mut self, row: u32) -> Option<&mut Vec<String>> {
        match &mut self.data.headers {
            Some(headers) if row == 0 => Some(headers),
            Some(_) => self.data.rows.get_mut((row - 1) as usize),
            None => self.data.rows.get_mut(row as usize),
        }
    }

    /// The cell text at `(row, col)`, if it exists.
    fn cell_at(&self, row: u32, col: u32) -> Option<&str> {
        self.row_cells(row)?.get(col as usize).map(String::as_str)
    }

    /// The header label of column `col`, if the source had headers.
    fn column_name(&self, col: u32) -> Option<&str> {
        self.data
            .headers
            .as_ref()?
            .get(col as usize)
            .map(String::as_str)
    }

    /// Re-serialize the rows to CSV bytes, honoring the original
    /// delimiter and trailing-newline.
    fn serialize(&self) -> Result<Vec<u8>> {
        let mut writer = csv::WriterBuilder::new()
            .delimiter(self.data.delimiter)
            .has_headers(false)
            .from_writer(Vec::new());
        if let Some(headers) = &self.data.headers {
            writer
                .write_record(headers)
                .map_err(|e| Error::new(ErrorKind::Validation, format!("CSV write: {e}")))?;
        }
        for row in &self.data.rows {
            writer
                .write_record(row)
                .map_err(|e| Error::new(ErrorKind::Validation, format!("CSV write: {e}")))?;
        }
        let mut bytes = writer
            .into_inner()
            .map_err(|e| Error::new(ErrorKind::Validation, format!("CSV flush: {e}")))?;
        // The csv crate writes CRLF; normalize to LF and honor the
        // original trailing-newline so a clean file round-trips exactly.
        bytes.retain(|&b| b != b'\r');
        if !self.data.trailing_newline && bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        Ok(bytes)
    }

    fn redact_one(
        &mut self,
        location: &TabularLocation,
        replacement: &TextReplacement,
    ) -> Result<()> {
        let Some(cell) = self
            .row_cells_mut(location.row_index)
            .and_then(|cells| cells.get_mut(location.column_index as usize))
        else {
            return Ok(());
        };
        let start = location.start_offset.unwrap_or(0);
        let end = location.end_offset.unwrap_or(cell.len());
        let value = replacement.value().unwrap_or_default();
        redact::replace_range(cell, value, start..end)
    }
}

impl Handler<Tabular> for CsvHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::new(bytes::Bytes::from(self.serialize()?)))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Tabular>>> {
        loop {
            if self.cursor.row >= self.total_rows() {
                return Ok(None);
            }
            let row = self.cursor.row;
            let col = self.cursor.col;
            if col as usize >= self.row_len(row).unwrap_or(0) {
                // End of this row: advance and retry (skips empty rows).
                self.cursor.row += 1;
                self.cursor.col = 0;
                continue;
            }
            self.cursor.col += 1;

            let cell = self
                .cell_at(row, col)
                .expect("bounds checked above")
                .to_owned();
            let mut location = TabularLocation::new(row, col);
            let mut hints = Vec::new();
            if let Some(name) = self.column_name(col) {
                location = location.with_column_name(name.to_owned());
                hints.push(name.to_owned());
            }
            return Ok(Some(Chunk {
                location,
                data: TextData::new(cell),
                hints,
            }));
        }
    }

    fn lift(&self, chunk: &Chunk<Tabular>, local: TabularLocation) -> Option<TabularLocation> {
        // `local` carries the chunk-local intra-cell byte range in its
        // offsets (its row/column are placeholders); a missing range means
        // the whole cell. Re-anchor onto the chunk's real cell coordinates.
        let cell = self.cell_at(chunk.location.row_index, chunk.location.column_index)?;
        let start = local.start_offset.unwrap_or(0);
        let end = local.end_offset.unwrap_or(cell.len());
        if start > end || end > cell.len() {
            return None;
        }
        let mut location =
            TabularLocation::new(chunk.location.row_index, chunk.location.column_index)
                .with_range(start, end);
        if let Some(name) = &chunk.location.column_name {
            location = location.with_column_name(name.clone());
        }
        if let Some(sheet) = &chunk.location.sheet_name {
            location = location.with_sheet_name(sheet.clone());
        }
        Some(location)
    }
}

impl DataReader<Tabular> for CsvHandler {
    async fn read_at(&self, location: &TabularLocation) -> Result<Option<TextData>> {
        let Some(cell) = self.cell_at(location.row_index, location.column_index) else {
            return Ok(None);
        };
        // An intra-cell range slices the cell; no range reads the whole cell.
        match (location.start_offset, location.end_offset) {
            (Some(start), Some(end)) => Ok(cell.get(start..end).map(TextData::new)),
            _ => Ok(Some(TextData::new(cell.to_owned()))),
        }
    }
}

impl DataWriter<Tabular> for CsvHandler {
    async fn write_at(&mut self, mut redactions: Redactions<Tabular>) -> Result<()> {
        // Sort by position, then apply right-to-left so an edit's length
        // delta doesn't shift earlier intra-cell offsets in the same cell.
        redactions.sort_by_position();
        let items: Vec<_> = redactions.iter().cloned().collect();
        for (location, replacement) in items.iter().rev() {
            self.redact_one(location, replacement)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::csv_loader::CsvLoader;
    use super::*;
    use crate::Loader;

    async fn load(text: &str) -> CsvHandler {
        CsvLoader::default()
            .decode(ContentData::from_text(text))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn streams_cells_row_major_with_headers() {
        let mut h = load("name,email\nAlice,a@x.test\n").await;
        let mut seen = Vec::new();
        while let Some(chunk) = h.read_next().await.unwrap() {
            seen.push((
                chunk.location.row_index,
                chunk.location.column_index,
                chunk.data.as_str().to_owned(),
            ));
        }
        assert_eq!(
            seen,
            vec![
                (0, 0, "name".into()),
                (0, 1, "email".into()),
                (1, 0, "Alice".into()),
                (1, 1, "a@x.test".into()),
            ]
        );
    }

    #[tokio::test]
    async fn read_at_slices_intra_cell_range() {
        let h = load("name\nAlice Carter\n").await;
        // Row 1, col 0 = "Alice Carter"; bytes 0..5 = "Alice".
        let loc = TabularLocation::new(1, 0).with_range(0, 5);
        assert_eq!(h.read_at(&loc).await.unwrap().unwrap().as_str(), "Alice");
    }

    #[tokio::test]
    async fn lift_maps_offsets_into_the_cell() {
        let mut h = load("name\nAlice Carter\n").await;
        let _hdr = h.read_next().await.unwrap().unwrap();
        let cell = h.read_next().await.unwrap().unwrap();
        // Chunk-local: row/col are placeholders, offsets carry the range.
        let local = TabularLocation::new(0, 0).with_range(6, 12);
        let lifted = h.lift(&cell, local).expect("in bounds");
        assert_eq!(lifted.row_index, 1);
        assert_eq!(lifted.column_index, 0);
        assert_eq!(lifted.start_offset, Some(6));
        assert_eq!(lifted.end_offset, Some(12));
        // Out-of-bounds range lifts to nothing.
        let oob = TabularLocation::new(0, 0).with_range(0, 99);
        assert!(h.lift(&cell, oob).is_none());
    }

    #[tokio::test]
    async fn redact_replaces_intra_cell_range_and_reencodes() {
        let mut h = load("name,email\nAlice,alice@x.test\n").await;
        let mut batch: Redactions<Tabular> = Redactions::new();
        // Row 1, col 1 = "alice@x.test"; replace the whole cell.
        batch.push(
            TabularLocation::new(1, 1),
            TextReplacement::substituted("[EMAIL]"),
        );
        h.write_at(batch).await.unwrap();
        let out = h.encode().unwrap();
        assert_eq!(out.decode().unwrap(), "name,email\nAlice,[EMAIL]\n");
    }

    #[tokio::test]
    async fn no_headers_addresses_rows_from_zero() {
        // Headerless data: row 0 is the first data row, not a header.
        let mut handler = CsvHandler::new(CsvData {
            headers: None,
            rows: vec![vec!["a".into(), "b".into()], vec!["c".into(), "d".into()]],
            delimiter: b',',
            trailing_newline: true,
        });
        let first = handler.read_next().await.unwrap().unwrap();
        assert_eq!(first.location.row_index, 0);
        assert_eq!(first.data.as_str(), "a");
        // No header row, so a cell carries no column name.
        assert!(first.location.column_name.is_none());
        assert_eq!(handler.encode().unwrap().decode().unwrap(), "a,b\nc,d\n");
    }
}
