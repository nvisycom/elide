//! XLSX handler: holds the workbook's cells and streams them one at a time,
//! with intra-cell random-access reads and redactions.
//!
//! A cell holds text, so a [`TabularReplacement`]'s cell treatment applies
//! through the shared text-redaction helper. Only the *location* is tabular: a
//! `(sheet, row, column)` address with an optional intra-cell byte range. On
//! [`encode`](Handler::encode) the edited cells become
//! [`CellEdit`](elide_office::xlsx::CellEdit)s and the workbook is re-packed
//! byte-faithfully by [`elide_office`], de-sharing any shared-string cell that
//! was redacted so other cells keep the pooled value.

use std::collections::{BTreeSet, HashMap};

use bytes::Bytes;
use elide_core::modality::tabular::{Tabular, TabularLocation, TabularReplacement};
use elide_core::modality::text::{TextData, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter, Hint};
use elide_core::operator::Redactions;
use elide_core::{Error, ErrorKind, Result};
use elide_office::xlsx::{CellEdit, Xlsx};

use super::xlsx_loader::XlsxLoader;
use crate::codec::{Container, Part};
use crate::content::ContentData;
use crate::handler::redact;
use crate::{Format, FormatId, Handler, LocalId};

/// Stable [`FormatId`] for the XLSX codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.tabular.xlsx");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Tabular, _>(FORMAT_ID.clone(), XlsxLoader)
        .with_extensions(["xlsx"])
        .with_content_types(["application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"])
}

/// One extracted cell: its sheet, zero-based coordinates, and current text.
#[derive(Debug, Clone)]
pub(crate) struct XlsxCell {
    /// Display name of the sheet the cell is on.
    pub(crate) sheet: String,
    /// Zero-based row index.
    pub(crate) row: u32,
    /// Zero-based column index.
    pub(crate) column: u32,
    /// The cell's text, mutated in place as redactions apply.
    pub(crate) text: String,
}

/// Handler for a decoded XLSX workbook. Each cell is independently addressable
/// via a [`TabularLocation`] scoped by sheet name.
///
/// A workbook also holds non-cell text — cell comments and drawing/chart text —
/// in its own XML parts. Those are surfaced through the [`Container`] surface as
/// `xml`-hinted parts the orchestrator drives with the markup pipeline, and their
/// redacted bytes fold back into the re-packed workbook on encode.
#[derive(Debug)]
pub(crate) struct XlsxHandler {
    /// The original package bytes, retained so [`elide_office`] re-packs every
    /// unedited part unchanged on encode.
    archive: Bytes,
    /// The workbook's text-bearing cells, in extraction order.
    cells: Vec<XlsxCell>,
    /// Streaming cursor into `cells`.
    cursor: usize,
    /// The non-cell text parts (comments, drawings, charts), path → bytes, cached
    /// at decode so the [`Container`] surface lists them without re-opening.
    text_parts: Vec<(String, Bytes)>,
    /// Redacted bytes for text parts, keyed by part path, filled through
    /// [`Container::replace_part`] and folded in on encode.
    replacements: HashMap<String, Bytes>,
    /// Indices into `cells` of the cells a redaction actually changed, so encode
    /// rewrites only those — leaving unedited shared-string cells shared.
    changed: BTreeSet<usize>,
}

impl XlsxHandler {
    /// Wrap the workbook's `archive` bytes, its extracted `cells`, and its
    /// non-cell `text_parts`.
    pub(crate) fn new(
        archive: Bytes,
        cells: Vec<XlsxCell>,
        text_parts: Vec<(String, Bytes)>,
    ) -> Self {
        Self {
            archive,
            cells,
            cursor: 0,
            text_parts,
            replacements: HashMap::new(),
            changed: BTreeSet::new(),
        }
    }

    /// The index of the unique cell at `(sheet, row, column)`.
    ///
    /// A location with no sheet name must still identify exactly one cell: if the
    /// same coordinates exist on more than one sheet, the match is ambiguous and
    /// this errs rather than silently editing the wrong sheet. `Ok(None)` means no
    /// cell matched.
    fn cell_index(&self, sheet: Option<&str>, row: u32, column: u32) -> Result<Option<usize>> {
        let mut matches = self.cells.iter().enumerate().filter(|(_, c)| {
            c.row == row && c.column == column && sheet.is_none_or(|name| c.sheet == name)
        });
        let first = matches.next().map(|(i, _)| i);
        if first.is_some() && matches.next().is_some() {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!(
                    "xlsx redaction at (row {row}, column {column}) is ambiguous \
                     across sheets; a sheet name is required"
                ),
            ));
        }
        Ok(first)
    }

    /// The header text for `column` on `sheet`: the text of the cell at row 0 of
    /// that column, if any. Provides column context to the recognizer.
    fn column_header(&self, sheet: &str, column: u32) -> Option<&str> {
        self.cells
            .iter()
            .find(|c| c.sheet == sheet && c.row == 0 && c.column == column)
            .map(|c| c.text.as_str())
    }

    /// The cell text at `location`, if a unique cell exists.
    fn cell_at(&self, location: &TabularLocation) -> Result<Option<&str>> {
        let sheet = location.sheet_name.as_deref();
        let Some(index) = self.cell_index(sheet, location.row_index, location.column_index)? else {
            return Ok(None);
        };
        Ok(Some(self.cells[index].text.as_str()))
    }

    /// Apply one cell edit at `location`, replacing its intra-cell range (or the
    /// whole cell when no range is set) with the replacement text.
    ///
    /// Fail-closed: a redaction that matches no cell is an error, not a silent
    /// no-op, so a request can never appear to succeed without changing the
    /// intended cell.
    fn redact_one(
        &mut self,
        location: &TabularLocation,
        replacement: &TextReplacement,
    ) -> Result<()> {
        let sheet = location.sheet_name.as_deref();
        let index = self
            .cell_index(sheet, location.row_index, location.column_index)?
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!(
                        "xlsx redaction targets no cell at sheet {:?} (row {}, column {})",
                        sheet, location.row_index, location.column_index
                    ),
                )
            })?;
        let cell = &mut self.cells[index];
        let start = location.start_offset.unwrap_or(0);
        let end = location.end_offset.unwrap_or(cell.text.len());
        let value = replacement.value().unwrap_or_default();
        redact::replace_range(&mut cell.text, value, start..end)?;
        // Only a cell that was actually edited is sent for rewrite, so unchanged
        // shared-string cells are not needlessly de-shared.
        self.changed.insert(index);
        Ok(())
    }
}

#[async_trait::async_trait]
impl Handler<Tabular> for XlsxHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        // Rewrite only the cells a redaction actually changed. Sending every cell
        // would de-share the whole workbook (each shared cell becomes an inline
        // string); sending just the edited ones de-shares only what was redacted
        // and leaves the shared-string table otherwise intact.
        let edits: Vec<CellEdit> = self
            .changed
            .iter()
            .map(|&index| {
                let cell = &self.cells[index];
                CellEdit::new(cell.sheet.clone(), cell.row, cell.column, cell.text.clone())
            })
            .collect();
        // The redacted non-cell text parts (comments, drawings, charts) fold in
        // alongside the cell edits for one byte-faithful re-pack.
        let part_edits: Vec<(String, Vec<u8>)> = self
            .replacements
            .iter()
            .map(|(path, bytes)| (path.clone(), bytes.to_vec()))
            .collect();
        let bytes = Xlsx::open(&self.archive)
            .and_then(|xlsx| xlsx.rewrite_with_parts(&edits, &part_edits))
            .map_err(xlsx_error)?;
        Ok(ContentData::new(Bytes::from(bytes)))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Tabular>>> {
        if self.cursor >= self.cells.len() {
            return Ok(None);
        }
        let cell = &self.cells[self.cursor];
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
            && let Some(header) = self.column_header(&cell.sheet, cell.column)
        {
            location = location.clone().with_column_name(header.to_owned());
            let header_location = TabularLocation::new(0, cell.column)
                .with_sheet_name(cell.sheet.clone())
                .with_column_name(header.to_owned());
            hints.push(Hint::new(header_location, TextData::new(header.to_owned())));
        }
        Ok(Some(Chunk {
            location,
            data: TextData::new(cell.text.clone()),
            hints,
        }))
    }

    fn lift(&self, chunk: &Chunk<Tabular>, local: TabularLocation) -> Option<TabularLocation> {
        // `local` carries the chunk-local intra-cell range in its offsets; its
        // row/column/sheet are placeholders. Re-anchor onto the chunk's cell. A
        // chunk always names its own sheet, so the lookup is unambiguous; treat
        // an ambiguous or missing match as no source pre-image.
        let cell = self.cell_at(&chunk.location).ok().flatten()?;
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

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        Some(self)
    }
}

impl Container for XlsxHandler {
    fn parts(&self) -> Vec<Part> {
        // Surface every non-cell text part (comments, drawings, charts) as an
        // `xml`-hinted blob the orchestrator decodes with the markup pipeline.
        self.text_parts
            .iter()
            .map(|(path, bytes)| Part {
                id: LocalId::new(path.clone()),
                bytes: bytes.clone(),
                hint: "xml".to_owned(),
            })
            .collect()
    }

    fn replace_part(&mut self, id: &LocalId, bytes: Bytes) -> Result<()> {
        // Only a part the workbook actually surfaced can be replaced, so a caller
        // can't smuggle bytes into a cell or structure part through this surface.
        if !self.text_parts.iter().any(|(path, _)| path == id.as_str()) {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("xlsx replace_part: `{id}` is not a text-bearing part"),
            ));
        }
        self.replacements.insert(id.as_str().to_owned(), bytes);
        Ok(())
    }
}

#[async_trait::async_trait]
impl DataReader<Tabular> for XlsxHandler {
    async fn read_at(&self, location: &TabularLocation) -> Result<Option<TextData>> {
        let Some(cell) = self.cell_at(location)? else {
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
            (None, _) => Ok(Some(TextData::new(cell.to_owned()))),
        }
    }
}

#[async_trait::async_trait]
impl DataWriter<Tabular> for XlsxHandler {
    async fn write_at(&mut self, mut redactions: Redactions<Tabular>) -> Result<()> {
        redactions.sort_by_position();
        // Right-to-left so an edit's length delta does not move earlier
        // intra-cell offsets in the same cell.
        for (location, replacement) in redactions.into_iter().rev() {
            match replacement {
                TabularReplacement::Cell(cell) => self.redact_one(&location, &cell)?,
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
        Ok(())
    }
}

/// Map an [`elide_office`] error into the codec's error type.
pub(crate) fn xlsx_error(err: elide_office::Error) -> Error {
    use elide_office::ErrorKind as OfficeKind;
    let kind = match err.kind() {
        OfficeKind::InvalidArchive | OfficeKind::InvalidPackage | OfficeKind::InvalidXml => {
            ErrorKind::MalformedInput
        }
        OfficeKind::UnsafeRewrite => ErrorKind::Processing,
        _ => ErrorKind::Processing,
    };
    Error::new(kind, err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Loader;

    /// The two-sheet workbook whose shared string is reused across sheets, for
    /// the de-share / cell-addressing mechanics tests.
    const SAMPLE: &[u8] =
        include_bytes!("../../../../elide-office/tests/testdata/shared_across_sheets.xlsx");

    /// A real Excel-authored workbook with a header row, for the column-context
    /// test.
    const REAL: &[u8] = include_bytes!("../../../../elide-office/tests/testdata/sample.xlsx");

    async fn load() -> XlsxHandler {
        XlsxLoader
            .decode(ContentData::new(Bytes::from_static(SAMPLE)))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn streams_cells_with_sheet_scoped_locations() {
        let mut h = load().await;
        let mut seen = Vec::new();
        while let Some(chunk) = h.read_next().await.unwrap() {
            seen.push((
                chunk.location.sheet_name.as_deref().map(str::to_owned),
                chunk.location.row_index,
                chunk.location.column_index,
                chunk.data.as_str().to_owned(),
            ));
        }
        assert!(seen.contains(&(
            Some("Customers".to_owned()),
            1,
            0,
            "alice@example.com".to_owned()
        )));
        assert!(seen.contains(&(Some("Customers".to_owned()), 1, 1, "123-45-6789".to_owned())));
        assert!(seen.contains(&(
            Some("Notes".to_owned()),
            0,
            0,
            "alice@example.com".to_owned()
        )));
    }

    #[tokio::test]
    async fn data_cells_carry_their_column_header_as_context() {
        // A data cell must stream with its header column's text, so a
        // context-gated pattern can match a value that carries no cue itself.
        let mut h = XlsxLoader
            .decode(ContentData::new(Bytes::from_static(REAL)))
            .await
            .unwrap();
        let mut card = None;
        while let Some(chunk) = h.read_next().await.unwrap() {
            if chunk.data.as_str() == "4111 1111 1111 1111" {
                card = Some(chunk);
                break;
            }
        }
        let card = card.expect("the card cell is streamed");
        // The header of the card's column is `card`, attached as the column name
        // and surfaced as a hint the recognizer can boost on.
        assert_eq!(card.location.column_name.as_deref(), Some("card"));
        assert!(
            card.hints.iter().any(|h| h.data.as_str() == "card"),
            "the header is surfaced as a context hint",
        );
        // A header cell is not re-hinted with itself.
        let mut header = XlsxLoader
            .decode(ContentData::new(Bytes::from_static(REAL)))
            .await
            .unwrap();
        while let Some(chunk) = header.read_next().await.unwrap() {
            if chunk.location.row_index == 0 {
                assert!(chunk.hints.is_empty(), "header cell has no self-hint");
                assert!(chunk.location.column_name.is_none());
            }
        }
    }

    #[tokio::test]
    async fn read_at_slices_intra_cell_range() {
        let h = load().await;
        // Customers!A2 = "alice@example.com"; bytes 0..5 = "alice".
        let loc = TabularLocation::new(1, 0)
            .with_sheet_name("Customers")
            .with_range(0, 5);
        assert_eq!(h.read_at(&loc).await.unwrap().unwrap().as_str(), "alice");
    }

    #[tokio::test]
    async fn redact_and_encode_removes_pii_and_de_shares() {
        let mut h = load().await;
        let mut batch: Redactions<Tabular> = Redactions::new();
        // Redact only Customers!A2 (alice). Notes!A1 shares the pool entry.
        batch.push(
            TabularLocation::new(1, 0).with_sheet_name("Customers"),
            TabularReplacement::Cell(TextReplacement::substituted("[EMAIL]")),
        );
        h.write_at(batch).await.unwrap();
        let out = h.encode().unwrap();

        // The redacted output is a valid workbook whose Customers!A2 is gone but
        // whose Notes!A1 (sharing the pool) still reads alice.
        let reopened = XlsxLoader
            .decode(ContentData::new(out.to_bytes()))
            .await
            .unwrap();
        let a2 = reopened
            .cell_at(&TabularLocation::new(1, 0).with_sheet_name("Customers"))
            .unwrap()
            .unwrap();
        assert_eq!(a2, "[EMAIL]");
        let notes = reopened
            .cell_at(&TabularLocation::new(0, 0).with_sheet_name("Notes"))
            .unwrap()
            .unwrap();
        assert_eq!(notes, "alice@example.com");
    }

    #[tokio::test]
    async fn structural_drops_are_refused() {
        let mut h = load().await;
        let mut batch: Redactions<Tabular> = Redactions::new();
        batch.push(
            TabularLocation::new(1, 0).with_sheet_name("Customers"),
            TabularReplacement::DropRow,
        );
        assert!(h.write_at(batch).await.is_err());
    }

    #[tokio::test]
    async fn encode_only_de_shares_redacted_cells() {
        let mut h = load().await;
        let mut batch: Redactions<Tabular> = Redactions::new();
        // Redact only Customers!A2 (alice). The `Email` header at A1 is a
        // different, unredacted shared string.
        batch.push(
            TabularLocation::new(1, 0).with_sheet_name("Customers"),
            TabularReplacement::Cell(TextReplacement::substituted("[EMAIL]")),
        );
        h.write_at(batch).await.unwrap();
        let out = h.encode().unwrap();

        let sheet1 = read_part(out.as_bytes(), "xl/worksheets/sheet1.xml");
        let sheet1 = String::from_utf8_lossy(&sheet1);
        // The redacted cell is de-shared to an inline string.
        assert!(
            sheet1.contains(r#"t="inlineStr"><is><t>[EMAIL]</t>"#),
            "{sheet1}"
        );
        // The untouched `Email` header cell stays a shared string — the workbook
        // is not wholesale de-shared.
        assert!(
            sheet1.contains(r#"<c r="A1" t="s"><v>0</v></c>"#),
            "unredacted shared cell was needlessly de-shared: {sheet1}"
        );
    }

    #[tokio::test]
    async fn a_sheetless_ambiguous_redaction_fails_closed() {
        // alice's coordinates (row 1, col 0) exist on Customers; a request with no
        // sheet name that matched two sheets would be ambiguous. Here the same
        // (0,0) exists on both Customers (Email) and Notes (alice), so a sheetless
        // request at (0,0) must fail rather than edit an arbitrary sheet.
        let mut h = load().await;
        let mut batch: Redactions<Tabular> = Redactions::new();
        batch.push(
            TabularLocation::new(0, 0), // no sheet name
            TabularReplacement::Cell(TextReplacement::substituted("[X]")),
        );
        assert!(h.write_at(batch).await.is_err());
    }

    const SAMPLE2: &[u8] = include_bytes!("../../../../elide-office/tests/testdata/sample2.xlsx");

    #[tokio::test]
    async fn exposes_non_cell_text_parts_as_xml_container_parts() {
        let mut h = XlsxLoader
            .decode(ContentData::new(Bytes::from_static(SAMPLE2)))
            .await
            .unwrap();
        let container = h.as_container_mut().expect("xlsx is a container");
        let parts = container.parts();
        // The comment and drawing parts are surfaced as xml-hinted blobs.
        assert!(
            parts
                .iter()
                .any(|p| p.id.as_str() == "xl/comments1.xml" && p.hint == "xml"),
            "comment part not surfaced: {parts:?}"
        );
        assert!(
            parts
                .iter()
                .any(|p| p.id.as_str() == "xl/drawings/drawing1.xml" && p.hint == "xml"),
        );
    }

    #[tokio::test]
    async fn replace_part_folds_redacted_text_into_the_encode() {
        let mut h = XlsxLoader
            .decode(ContentData::new(Bytes::from_static(SAMPLE2)))
            .await
            .unwrap();
        let redacted = Bytes::from_static(
            br#"<?xml version="1.0"?><comments><commentList><comment ref="A1"><text><r><t>[EMAIL]</t></r></text></comment></commentList></comments>"#,
        );
        {
            let container = h.as_container_mut().unwrap();
            container
                .replace_part(&LocalId::new("xl/comments1.xml"), redacted.clone())
                .unwrap();
            // An id the workbook does not surface is refused.
            assert!(
                container
                    .replace_part(&LocalId::new("xl/nope.xml"), redacted.clone())
                    .is_err()
            );
        }
        let out = h.encode().unwrap();
        let comment = read_part(out.as_bytes(), "xl/comments1.xml");
        assert!(String::from_utf8_lossy(&comment).contains("[EMAIL]"));
        assert!(!String::from_utf8_lossy(&comment).contains("carol@example.com"));
    }

    fn read_part(bytes: &[u8], name: &str) -> Vec<u8> {
        elide_office::opc::test_util::read_part(bytes, name)
            .unwrap_or_else(|| panic!("part `{name}` present in package"))
    }
}
