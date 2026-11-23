//! CSV handler: holds parsed rows and streams them cell by cell, with
//! intra-cell random-access reads and redactions.
//!
//! Cells hold text, so the handler applies a [`TabularReplacement`]'s cell
//! treatment through the shared text-redaction helper, and its structural
//! treatments, dropping a whole row or column, by removing them from the
//! parsed rows. Only the *location* is tabular: a `(row, column)` address
//! plus an optional intra-cell byte range.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::ops::Range;

use csv::Terminator;
use elide_codec::content::ContentData;
use elide_codec::string::{DisjointSpans, RedactRange};
use elide_codec::{Encode, Format, FormatId, Stream};
use elide_core::modality::tabular::{Tabular, TabularLocation, TabularReplacement};
use elide_core::modality::text::{TextData, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter, ResolvedHint};
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use super::loader::CsvLoader;
use crate::control;
use crate::slice::slice;

/// Stable [`FormatId`] for the CSV codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.tabular.csv");

/// The CSV [`Format`] with a header row and an auto-detected delimiter.
///
/// Use [`format_with`] to override either for a headerless file or a
/// non-comma delimiter (TSV, semicolon).
pub fn format() -> Format {
    format_from(CsvLoader::default())
}

/// The CSV [`Format`] with explicit parsing options: header and delimiter.
///
/// `has_headers` chooses whether the first row is the header (its default,
/// `true`). A header row enters the detection stream like any other and gives
/// the cells below it their column-name context hints; a headerless file wants
/// `false` so its first row is read as data.
///
/// `delimiter` sets the field separator; `None` auto-detects it (a comma when
/// nothing stands out), which can misread a TSV or semicolon-delimited file, so
/// pass `Some(b'\t')` / `Some(b';')` when the format is known.
pub fn format_with(has_headers: bool, delimiter: Option<u8>) -> Format {
    format_from(CsvLoader::new(has_headers, delimiter))
}

/// Build the CSV [`Format`] from a configured loader.
fn format_from(loader: CsvLoader) -> Format {
    Format::with_document_loader(FORMAT_ID, loader)
        .with_extensions(["csv"])
        .with_content_types(["text/csv"])
}

/// Parsed CSV content: an optional header row plus the data rows, with the
/// delimiter and line endings to write them back with. Quoting is normalized
/// on write: a field is quoted only where it needs to be.
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
    /// Whether the original's records end in `\r\n` rather than `\n`.
    pub crlf: bool,
}

impl CsvData {
    /// Every cell, the header row's first.
    fn cells_mut(&mut self) -> impl Iterator<Item = &mut String> {
        self.headers
            .iter_mut()
            .flatten()
            .chain(self.rows.iter_mut().flatten())
    }

    /// Strip the terminal control sequences from every cell, returning how
    /// many went.
    pub(super) fn strip_controls(&mut self) -> usize {
        self.cells_mut()
            .map(|cell| {
                let (stripped, removed) = control::strip_owned(mem::take(cell));
                *cell = stripped;
                removed
            })
            .sum()
    }
}

/// Handler for parsed CSV content. Each cell is independently addressable
/// via a [`TabularLocation`].
#[derive(Debug)]
pub(crate) struct CsvHandler {
    data: CsvData,
}

impl CsvHandler {
    /// Wrap parsed CSV data.
    pub(crate) fn new(data: CsvData) -> Self {
        Self { data }
    }

    /// Total number of addressable rows, header included.
    fn total_rows(&self) -> u32 {
        let header = u32::from(self.data.headers.is_some());
        header + self.data.rows.len() as u32
    }

    /// Number of cells in `row`, or `None` when the row is out of range.
    fn row_len(&self, row: u32) -> Option<usize> {
        self.row_cells(row).map(<[String]>::len)
    }

    /// The cells of `row` (header row 0 when present, then data rows).
    fn row_cells(&self, row: u32) -> Option<&[String]> {
        match &self.data.headers {
            Some(headers) if row == 0 => Some(headers),
            Some(_) => self.data.rows.get((row - 1) as usize),
            None => self.data.rows.get(row as usize),
        }
        .map(Vec::as_slice)
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

    /// Re-serialize the rows to CSV bytes with the original delimiter, line
    /// endings and trailing newline.
    fn serialize(&self) -> Result<Vec<u8>> {
        let terminator = if self.data.crlf {
            Terminator::CRLF
        } else {
            Terminator::Any(b'\n')
        };
        let mut writer = csv::WriterBuilder::new()
            .delimiter(self.data.delimiter)
            .terminator(terminator)
            .has_headers(false)
            // Rows may differ in width, as the loader reads them.
            .flexible(true)
            .from_writer(Vec::new());
        let write_error = |e: csv::Error| Error::processing(format!("CSV write: {e}"));
        if let Some(headers) = &self.data.headers {
            writer.write_record(headers).map_err(write_error)?;
        }
        for row in &self.data.rows {
            writer.write_record(row).map_err(write_error)?;
        }
        let mut bytes = writer
            .into_inner()
            .map_err(|e| Error::processing(format!("CSV flush: {e}")))?;
        if !self.data.trailing_newline {
            let ending: &[u8] = if self.data.crlf { b"\r\n" } else { b"\n" };
            if bytes.ends_with(ending) {
                bytes.truncate(bytes.len() - ending.len());
            }
        }
        Ok(bytes)
    }

    /// Apply `redactions`: cell edits, then column and row drops.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] for a location naming a sheet, a cell, span, row or
    /// column the table doesn't have, or two edits of one cell that overlap, leaving the table partly
    /// edited.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn apply(&mut self, mut redactions: Redactions<Tabular>) -> Result<()> {
        for (location, _) in redactions.iter() {
            unsheeted(location)?;
        }
        redactions.sort_by_position();
        self.ensure_disjoint_cells(&redactions)?;

        // Apply cell edits first, before any structural drop shifts indices.
        // Right-to-left so an edit's length delta doesn't move earlier
        // intra-cell offsets in the same cell. Collect drop targets as we go.
        let mut drop_rows = BTreeSet::new();
        let mut drop_cols = BTreeSet::new();
        for (location, replacement) in redactions.iter().rev() {
            match replacement {
                TabularReplacement::CellRedacted(cell) => self.redact_one(location, cell)?,
                TabularReplacement::RowDropped => {
                    drop_rows.insert(location.row_index);
                }
                TabularReplacement::ColumnDropped => {
                    drop_cols.insert(location.column_index);
                }
            }
        }

        // Then delete structure, highest index first so each removal leaves
        // the not-yet-removed indices valid: columns right-to-left, rows
        // bottom-up.
        for &col in drop_cols.iter().rev() {
            self.drop_column(col)?;
        }
        for &row in drop_rows.iter().rev() {
            self.drop_row(row)?;
        }
        Ok(())
    }

    /// Refuse a batch with two edits of one cell that overlap, a whole-cell
    /// edit counting as the cell's full span. Applied right to left, disjoint
    /// edits each meet the cell as it was up to their end.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] for a cell the table doesn't have, or an overlapping
    /// pair.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn ensure_disjoint_cells(&self, redactions: &Redactions<Tabular>) -> Result<()> {
        let mut cells: BTreeMap<(u32, u32), Vec<Range<usize>>> = BTreeMap::new();
        for (location, replacement) in redactions.iter() {
            if !matches!(replacement, TabularReplacement::CellRedacted(_)) {
                continue;
            }
            let cell = self
                .cell_at(location.row_index, location.column_index)
                .ok_or_else(|| no_cell(location))?;
            cells
                .entry((location.row_index, location.column_index))
                .or_default()
                .push(location.span.clone().unwrap_or(0..cell.len()));
        }
        cells.values().try_for_each(|spans| spans.ensure_disjoint())
    }

    /// Splice `replacement` over the cell (or the span within it) `location`
    /// names. A cell or span the table doesn't have is an error: skipping it
    /// would leave the content unredacted.
    fn redact_one(
        &mut self,
        location: &TabularLocation,
        replacement: &TextReplacement,
    ) -> Result<()> {
        let cell = self
            .row_cells_mut(location.row_index)
            .and_then(|cells| cells.get_mut(location.column_index as usize))
            .ok_or_else(|| no_cell(location))?;
        let span = location.span.clone().unwrap_or(0..cell.len());
        cell.redact_with(replacement, span)
    }

    /// Drop the row at `row_index` (the same addressing as [`row_cells`]).
    /// Dropping the header row writes the table without one.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if there is no such row.
    ///
    /// [`row_cells`]: Self::row_cells
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn drop_row(&mut self, row_index: u32) -> Result<()> {
        let data_index = match &self.data.headers {
            Some(_) if row_index == 0 => {
                self.data.headers = None;
                return Ok(());
            }
            Some(_) => (row_index - 1) as usize,
            None => row_index as usize,
        };
        if data_index >= self.data.rows.len() {
            return Err(Error::malformed(format!("no CSV row {row_index} to drop")));
        }
        self.data.rows.remove(data_index);
        Ok(())
    }

    /// Drop column `col` from the header and every data row that has it.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if no row has the column.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn drop_column(&mut self, col: u32) -> Result<()> {
        let index = col as usize;
        let mut dropped = false;
        for row in self.data.headers.iter_mut().chain(&mut self.data.rows) {
            if index < row.len() {
                row.remove(index);
                dropped = true;
            }
        }
        if !dropped {
            return Err(Error::malformed(format!("no CSV column {col} to drop")));
        }
        Ok(())
    }
}

impl Stream<Tabular> for CsvHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Tabular>>> {
        let mut chunks = Vec::new();
        for row in 0..self.total_rows() {
            for col in 0..self.row_len(row).unwrap_or(0) as u32 {
                let cell = self
                    .cell_at(row, col)
                    .expect("bounds checked above")
                    .to_owned();
                let mut location = TabularLocation::new(row, col);
                let mut hints = Vec::new();
                if let Some(name) = self.column_name(col) {
                    location = location.with_column_name(name.to_owned());
                    // A data cell's label is the header cell above it, a real
                    // tabular location the boost can point at.
                    if row > 0 {
                        let header_location =
                            TabularLocation::new(0, col).with_column_name(name.to_owned());
                        hints.push(ResolvedHint::new(
                            header_location,
                            TextData::new(name.to_owned()),
                        ));
                    }
                }
                chunks.push(Chunk {
                    location,
                    data: TextData::new(cell),
                    hints,
                });
            }
        }
        Ok(chunks)
    }

    fn lift(&self, chunk: &Chunk<Tabular>, local: TabularLocation) -> Option<TabularLocation> {
        // `local` carries the chunk-local intra-cell span (its row/column are
        // placeholders); a missing span means the whole cell. Re-anchor onto
        // the chunk's real cell coordinates.
        let cell = self.cell_at(chunk.location.row_index, chunk.location.column_index)?;
        let span = local.span.unwrap_or(0..cell.len());
        if span.start > span.end || span.end > cell.len() {
            return None;
        }
        Some(chunk.location.clone().with_span(span))
    }
}

impl Encode for CsvHandler {
    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::from(self.serialize()?))
    }
}

#[async_trait::async_trait]
impl DataReader<Tabular> for CsvHandler {
    async fn read_at(&self, location: &TabularLocation) -> Result<TextData> {
        unsheeted(location)?;
        let cell = self
            .cell_at(location.row_index, location.column_index)
            .ok_or_else(|| no_cell(location))?;
        // An intra-cell span slices the cell; no span reads the whole cell.
        match &location.span {
            Some(span) => slice(cell, span).map(TextData::new),
            None => Ok(TextData::new(cell.to_owned())),
        }
    }
}

#[async_trait::async_trait]
impl DataWriter<Tabular> for CsvHandler {
    async fn write_at(&mut self, redactions: Redactions<Tabular>) -> Result<()> {
        // Apply to a copy, kept only once the whole batch applies.
        let mut staged = Self::new(self.data.clone());
        staged.apply(redactions)?;
        *self = staged;
        Ok(())
    }
}

/// Refuse a location that names a sheet: a CSV file is one table, and the
/// location addresses a table this isn't.
fn unsheeted(location: &TabularLocation) -> Result<()> {
    match &location.sheet_name {
        Some(sheet) => Err(Error::malformed(format!(
            "CSV has no sheets, but the location names sheet {sheet:?}"
        ))),
        None => Ok(()),
    }
}

/// The error for a location naming a cell the table doesn't have.
fn no_cell(location: &TabularLocation) -> Error {
    Error::malformed(format!(
        "no CSV cell at row {}, column {}",
        location.row_index, location.column_index
    ))
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::primitive::Color;
    use elide_core::redaction::HiddenContent;

    use super::super::loader::CsvLoader;
    use super::*;

    async fn load(text: &str) -> CsvHandler {
        CsvLoader::default()
            .load(ContentData::from_text(text))
            .unwrap()
            .0
    }

    #[tokio::test]
    async fn streams_cells_row_major_with_headers() {
        let h = load("name,email\nAlice,a@x.test\n").await;
        let mut seen = Vec::new();
        for chunk in h.chunks().unwrap() {
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
        let loc = TabularLocation::new(1, 0).with_span(0..5);
        assert_eq!(h.read_at(&loc).await.unwrap().as_str(), "Alice");
    }

    #[tokio::test]
    async fn lift_maps_offsets_into_the_cell() {
        let h = load("name\nAlice Carter\n").await;
        let chunks = h.chunks().unwrap();
        let cell = &chunks[1];
        // Chunk-local: row/col are placeholders, offsets carry the range.
        let local = TabularLocation::new(0, 0).with_span(6..12);
        let lifted = h.lift(cell, local).expect("in bounds");
        assert_eq!(lifted.row_index, 1);
        assert_eq!(lifted.column_index, 0);
        assert_eq!(lifted.span, Some(6..12));
        // Out-of-bounds range lifts to nothing.
        let oob = TabularLocation::new(0, 0).with_span(0..99);
        assert!(h.lift(cell, oob).is_none());
    }

    #[tokio::test]
    async fn redact_replaces_intra_cell_range_and_reencodes() {
        let mut h = load("name,email\nAlice,alice@x.test\n").await;
        // Row 1, col 1 = "alice@x.test"; replace the whole cell.
        h.write_at(Redactions::from([(
            TabularLocation::new(1, 1),
            TabularReplacement::CellRedacted(TextReplacement::replaced("[EMAIL]")),
        )]))
        .await
        .unwrap();
        let out = h.encode().unwrap();
        assert_eq!(out.decode().unwrap(), "name,email\nAlice,[EMAIL]\n");
    }

    /// A covered cell is written as one mark per character it held.
    #[tokio::test]
    async fn a_covered_cell_is_marked_per_character() {
        let mut h = load("name,email\nAlice,alice@x.test\n").await;
        h.write_at(Redactions::from([(
            TabularLocation::new(1, 1),
            TabularReplacement::CellRedacted(TextReplacement::covered(Color::BLACK)),
        )]))
        .await
        .unwrap();
        let out = h.encode().unwrap();
        assert_eq!(out.decode().unwrap(), "name,email\nAlice,████████████\n");
    }

    #[tokio::test]
    async fn drop_row_removes_the_record() {
        let mut h = load("name,city\nAlice,NYC\nBob,LA\n").await;
        // A match anywhere in Bob's row (row 2) drops the whole record.
        h.write_at(Redactions::from([(
            TabularLocation::new(2, 0),
            TabularReplacement::RowDropped,
        )]))
        .await
        .unwrap();
        let out = h.encode().unwrap();
        assert_eq!(out.decode().unwrap(), "name,city\nAlice,NYC\n");
    }

    #[tokio::test]
    async fn drop_multiple_rows_stays_aligned() {
        let mut h = load("n\nA\nB\nC\nD\n").await;
        // Drop rows 1 (A) and 3 (C); bottom-up removal keeps indices valid.
        h.write_at(Redactions::from([
            (TabularLocation::new(1, 0), TabularReplacement::RowDropped),
            (TabularLocation::new(3, 0), TabularReplacement::RowDropped),
        ]))
        .await
        .unwrap();
        assert_eq!(h.encode().unwrap().decode().unwrap(), "n\nB\nD\n");
    }

    #[tokio::test]
    async fn drop_row_drops_the_header_too() {
        let mut h = load("name,city\nAlice,NYC\nBob,LA\n").await;
        h.write_at(Redactions::from([
            (TabularLocation::new(0, 0), TabularReplacement::RowDropped),
            (TabularLocation::new(2, 0), TabularReplacement::RowDropped),
        ]))
        .await
        .unwrap();
        assert_eq!(h.encode().unwrap().decode().unwrap(), "Alice,NYC\n");
    }

    #[tokio::test]
    async fn drop_column_removes_header_and_every_cell() {
        let mut h = load("name,ssn,city\nAlice,111,NYC\nBob,222,LA\n").await;
        // A match in the ssn column (col 1) drops it across all rows.
        h.write_at(Redactions::from([(
            TabularLocation::new(1, 1),
            TabularReplacement::ColumnDropped,
        )]))
        .await
        .unwrap();
        assert_eq!(
            h.encode().unwrap().decode().unwrap(),
            "name,city\nAlice,NYC\nBob,LA\n"
        );
    }

    #[tokio::test]
    async fn no_headers_addresses_rows_from_zero() {
        // Headerless data: row 0 is the first data row, not a header.
        let handler = CsvHandler::new(CsvData {
            headers: None,
            rows: vec![vec!["a".into(), "b".into()], vec!["c".into(), "d".into()]],
            delimiter: b',',
            trailing_newline: true,
            crlf: false,
        });
        let first = handler.chunks().unwrap().into_iter().next().unwrap();
        assert_eq!(first.location.row_index, 0);
        assert_eq!(first.data.as_str(), "a");
        // No header row, so a cell carries no column name.
        assert!(first.location.column_name.is_none());
        assert_eq!(handler.encode().unwrap().decode().unwrap(), "a,b\nc,d\n");
    }

    #[tokio::test]
    async fn options_disable_headers_so_the_first_row_is_data() {
        // With `has_headers = false`, row 0 is an ordinary data row.
        let (mut handler, _) = CsvLoader::new(false, None)
            .load(ContentData::from_text("a@x.test,b\nc,d\n"))
            .unwrap();
        let first = handler.chunks().unwrap().into_iter().next().unwrap();
        assert_eq!(first.location.row_index, 0);
        assert!(first.location.column_name.is_none());
        handler
            .write_at(Redactions::from([(
                TabularLocation::new(0, 0),
                TabularReplacement::RowDropped,
            )]))
            .await
            .unwrap();
        assert_eq!(handler.encode().unwrap().decode().unwrap(), "c,d\n");
    }

    #[tokio::test]
    async fn options_set_the_delimiter_the_sniffer_would_miss() {
        // A single-column-looking TSV: auto-detect would read it as one comma
        // field; an explicit tab splits it into two columns.
        let (handler, _) = CsvLoader::new(true, Some(b'\t'))
            .load(ContentData::from_text("name\temail\nAlice\ta@x.test\n"))
            .unwrap();
        assert_eq!(handler.row_cells(1).unwrap(), &["Alice", "a@x.test"]);
    }

    /// A control sequence in a cell goes at decode, and the strip is recorded;
    /// a formula stays as written.
    #[test]
    fn terminal_control_sequences_are_stripped() -> Result<()> {
        let src = "name,note\nJa\u{1b}[mne Roe,=SUM(1;2)\n";
        let (handler, stripped) = CsvLoader::default().load(ContentData::from_text(src))?;
        assert_eq!(
            handler.encode()?.decode()?,
            "name,note\nJane Roe,=SUM(1;2)\n"
        );
        let stripped: Vec<_> = stripped
            .iter()
            .map(|entry| (entry.content, entry.count))
            .collect();
        assert_eq!(stripped, [(HiddenContent::Undisplayed, 1)]);
        Ok(())
    }

    /// A stripped table whose rows differ in width is written back as read.
    #[test]
    fn a_stripped_ragged_table_encodes() -> Result<()> {
        let src = "a,b,c\nJa\u{1b}[mne\nx,y\n";
        let (handler, _) = CsvLoader::default().load(ContentData::from_text(src))?;
        assert_eq!(handler.encode()?.decode()?, "a,b,c\nJane\nx,y\n");
        Ok(())
    }

    #[tokio::test]
    async fn crlf_line_endings_round_trip() {
        let src = "name,email\r\nAlice,a@x.test\r\n";
        let h = load(src).await;
        assert_eq!(h.encode().unwrap().as_bytes(), src.as_bytes());
    }

    /// The line ending is the one closing the first record, not a line break
    /// inside a quoted field before it.
    #[tokio::test]
    async fn a_quoted_line_break_does_not_set_the_line_ending() {
        for src in [
            "\"first\nname\",email\r\nAlice,a@x.test\r\n",
            "\"first\r\nname\",email\nAlice,a@x.test\n",
        ] {
            let h = load(src).await;
            assert_eq!(h.encode().unwrap().as_bytes(), src.as_bytes(), "{src:?}");
        }
    }

    /// A `\r` inside a quoted cell is content, kept on write.
    #[tokio::test]
    async fn a_carriage_return_inside_a_cell_is_kept() {
        let src = "name,note\nAlice,\"line one\r\nline two\"\n";
        let h = load(src).await;
        assert_eq!(h.cell_at(1, 1), Some("line one\r\nline two"));
        assert_eq!(h.encode().unwrap().as_bytes(), src.as_bytes());
    }

    #[tokio::test]
    async fn no_trailing_newline_round_trips() {
        let h = load("a,b\r\nc,d").await;
        assert_eq!(h.encode().unwrap().as_bytes(), b"a,b\r\nc,d");
    }

    /// A cell, span, row or column the table doesn't have fails the batch
    /// rather than being skipped, which would leave the content unredacted.
    #[tokio::test]
    async fn a_location_outside_the_table_is_an_error() {
        let replaced = || TabularReplacement::CellRedacted(TextReplacement::replaced("[X]"));
        let batches = [
            (TabularLocation::new(9, 0), replaced()),
            (TabularLocation::new(1, 9), replaced()),
            (TabularLocation::new(1, 0).with_span(0..99), replaced()),
            (TabularLocation::new(9, 0), TabularReplacement::RowDropped),
            (
                TabularLocation::new(1, 9),
                TabularReplacement::ColumnDropped,
            ),
        ];
        let src = "name,city\nAlice,NYC\n";
        for (location, replacement) in batches {
            let mut h = load(src).await;
            // Valid edits in the same batch are not kept either.
            let error = h
                .write_at(Redactions::from([
                    (location.clone(), replacement),
                    (TabularLocation::new(1, 0), replaced()),
                    (
                        TabularLocation::new(0, 1),
                        TabularReplacement::ColumnDropped,
                    ),
                ]))
                .await
                .expect_err("an unwritable location must fail");
            assert_eq!(error.kind(), ErrorKind::MalformedInput, "{location:?}");
            assert_eq!(
                h.encode().unwrap().as_bytes(),
                src.as_bytes(),
                "{location:?}"
            );
        }
    }

    #[tokio::test]
    async fn overlapping_edits_of_one_cell_are_an_error() {
        let replaced =
            |value: &str| TabularReplacement::CellRedacted(TextReplacement::replaced(value));
        let src = "id\nabcdef\n";
        let mut h = load(src).await;
        let error = h
            .write_at(Redactions::from([
                (TabularLocation::new(1, 0).with_span(0..7), replaced("[A]")),
                (
                    TabularLocation::new(1, 0).with_span(3..6),
                    replaced("123456789"),
                ),
            ]))
            .await
            .expect_err("overlapping edits of one cell must fail");
        assert_eq!(error.kind(), ErrorKind::MalformedInput);
        assert_eq!(h.encode().unwrap().as_bytes(), src.as_bytes());

        // A whole-cell edit overlaps any span of the cell; touching spans apply.
        let error = h
            .write_at(Redactions::from([
                (TabularLocation::new(1, 0), replaced("[A]")),
                (TabularLocation::new(1, 0).with_span(3..6), replaced("[B]")),
            ]))
            .await
            .expect_err("a whole-cell edit overlaps a span of it");
        assert_eq!(error.kind(), ErrorKind::MalformedInput);
        h.write_at(Redactions::from([
            (TabularLocation::new(1, 0).with_span(0..3), replaced("[A]")),
            (TabularLocation::new(1, 0).with_span(3..6), replaced("[B]")),
        ]))
        .await
        .unwrap();
        assert_eq!(h.encode().unwrap().as_bytes(), b"id\n[A][B]\n");
    }

    /// A CSV file is one table: a location naming a sheet addresses another.
    #[tokio::test]
    async fn a_location_naming_a_sheet_is_an_error() {
        let replaced =
            |value: &str| TabularReplacement::CellRedacted(TextReplacement::replaced(value));
        let src = "id\nabcdef\n";
        let mut h = load(src).await;
        let sheeted = TabularLocation::new(1, 0).with_sheet_name("Z");
        let error = h.read_at(&sheeted).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedInput);
        let error = h
            .write_at(Redactions::from([
                (sheeted.with_span(0..1), replaced("LONG")),
                (TabularLocation::new(1, 0).with_span(3..6), replaced("X")),
            ]))
            .await
            .expect_err("a sheet-qualified location must fail");
        assert_eq!(error.kind(), ErrorKind::MalformedInput);
        assert_eq!(h.encode().unwrap().as_bytes(), src.as_bytes());
    }

    #[tokio::test]
    async fn only_data_cells_carry_the_header_hint() {
        let h = load("name\nAlice\n").await;
        let chunks = h.chunks().unwrap();
        assert!(chunks[0].hints.is_empty());
        assert_eq!(chunks[1].hints.len(), 1);
    }

    #[test]
    fn format_with_registers_the_same_extension_and_content_type() {
        let a = super::format();
        let b = super::format_with(false, Some(b';'));
        assert_eq!(a.extensions(), b.extensions());
        assert_eq!(a.content_types(), b.content_types());
    }
}
