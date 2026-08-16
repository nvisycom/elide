//! Parsing a worksheet part into cells, each carrying the byte spans a
//! redaction needs.
//!
//! A cell is `<c r="A1" t="..">…</c>`. Two string kinds hold user text:
//! `t="s"` names a shared string by the index in its `<v>`, and
//! `t="inlineStr"` carries the text directly in `<is><t>..</t></is>`. Every
//! other cell (numbers, booleans, cached formula values) holds no free text and
//! is skipped. Row and column come from the `r` reference, so a sparse sheet is
//! addressed correctly.

use std::ops::Range;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{Error, Result};
use crate::xlsx::cellref::parse_cell_ref;

/// Where a cell's text lives, and the byte spans to rewrite it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CellSource {
    /// A shared string: the index into the shared-string table, plus the byte
    /// range of the whole `<c>…</c>` element (redaction de-shares the cell by
    /// replacing that element with an inline string).
    Shared {
        /// Index into the shared-string table.
        index: usize,
        /// Byte range of the whole `<c …>…</c>` element in the sheet's bytes.
        cell: Range<usize>,
        /// The cell's attributes other than `t` (e.g. `r`, `s`), verbatim, so a
        /// de-share preserves them and only swaps the type and body.
        attributes: String,
    },
    /// An inline string: the byte range of the inner text of its `<t>` element,
    /// spliced in place.
    Inline {
        /// Byte range of the `<t>` inner text in the sheet's bytes.
        text: Range<usize>,
    },
}

/// One text-bearing cell of a worksheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SheetCell {
    /// Zero-based row index (from the cell's `r` reference).
    pub(crate) row: u32,
    /// Zero-based column index (from the cell's `r` reference).
    pub(crate) column: u32,
    /// The shared-string index for a `t="s"` cell, so extraction can resolve
    /// the text; `None` for an inline cell whose text is read from `text_span`.
    pub(crate) shared_index: Option<usize>,
    /// The byte range of the inline text, for a `t="inlineStr"` cell; `None`
    /// for a shared cell.
    pub(crate) inline_text: Option<Range<usize>>,
    /// The cell's byte span and attributes, retained for a de-share rewrite.
    pub(crate) source: CellSource,
}

/// The text-bearing cells of a worksheet part `raw`, in document order.
///
/// quick-xml reports byte positions relative to the text after a leading BOM, so
/// every recorded span is shifted back onto the original bytes.
pub(crate) fn parse_cells(raw: &str) -> Result<Vec<SheetCell>> {
    let malformed = |e: quick_xml::Error| Error::invalid_xml(format!("worksheet malformed: {e}"));
    let mut reader = Reader::from_str(raw);
    let bom = bom_len(raw);
    let mut last = bom;
    let mut cells = Vec::new();
    let mut open: Option<OpenCell> = None;

    loop {
        let event = reader.read_event().map_err(malformed)?;
        let span = last..reader.buffer_position() as usize + bom;
        last = span.end;

        match event {
            Event::Eof => break,
            Event::Start(e) if e.local_name().as_ref() == b"c" => {
                open = Some(OpenCell::parse(&e, span.start)?);
            }
            // A self-closing `<c r=".." />` holds no value; nothing to redact.
            Event::Empty(e) if e.local_name().as_ref() == b"c" => {
                open = None;
                let _ = OpenCell::parse(&e, span.start)?;
            }
            Event::Start(e) if e.local_name().as_ref() == b"t" => {
                if let Some(cell) = open.as_mut() {
                    cell.text_open = Some(span.end);
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"t" => {
                if let Some(cell) = open.as_mut()
                    && let Some(text_start) = cell.text_open.take()
                {
                    cell.inline_text = Some(text_start..span.start);
                }
            }
            Event::Text(t) if open.as_ref().and_then(|c| c.value_open).is_some() => {
                // The `<v>` of a shared cell holds the shared-string index.
                if let Some(cell) = open.as_mut() {
                    cell.value_text = Some(String::from_utf8_lossy(t.as_ref()).into_owned());
                }
            }
            Event::Start(e) if e.local_name().as_ref() == b"v" => {
                if let Some(cell) = open.as_mut() {
                    cell.value_open = Some(span.end);
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"v" => {
                if let Some(cell) = open.as_mut() {
                    cell.value_open = None;
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"c" => {
                if let Some(cell) = open.take()
                    && let Some(finished) = cell.finish(span.end)?
                {
                    cells.push(finished);
                }
            }
            _ => {}
        }
    }
    Ok(cells)
}

/// A cell being assembled as its child events stream in.
struct OpenCell {
    row: u32,
    column: u32,
    cell_type: CellType,
    /// Byte offset where the `<c …>` element begins.
    start: usize,
    /// Attributes other than `t`, verbatim, for a de-share rewrite.
    attributes: String,
    /// Byte offset just past a `<t>` open, while inside it.
    text_open: Option<usize>,
    /// The `<t>` inner text byte range, once closed.
    inline_text: Option<Range<usize>>,
    /// Byte offset just past a `<v>` open, while inside it.
    value_open: Option<usize>,
    /// The `<v>` text (a shared-string index for a `t="s"` cell).
    value_text: Option<String>,
}

/// The declared type of a cell's `t` attribute, narrowed to what carries text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellType {
    /// `t="s"`: a shared-string index.
    Shared,
    /// `t="inlineStr"`: text in `<is><t>`.
    Inline,
    /// Anything else (number, boolean, cached formula string, …): no free text.
    Other,
}

impl OpenCell {
    /// Read a `<c>` start tag's `r`, `t`, and other attributes.
    fn parse(e: &quick_xml::events::BytesStart<'_>, start: usize) -> Result<Self> {
        let mut reference = None;
        let mut cell_type = CellType::Other;
        let mut attributes = String::new();
        for attr in e.attributes() {
            let attr = attr.map_err(|err| Error::invalid_xml(format!("cell attribute: {err}")))?;
            let local = attr.key.local_name();
            if local.as_ref() == b"t" {
                cell_type = match attr.value.as_ref() {
                    b"s" => CellType::Shared,
                    b"inlineStr" => CellType::Inline,
                    _ => CellType::Other,
                };
                // The type attribute is not carried: a de-share sets its own
                // `t="inlineStr"`.
                continue;
            }
            if local.as_ref() == b"r" {
                reference = Some(String::from_utf8_lossy(&attr.value).into_owned());
            }
            // Every non-`t` attribute (including `r` and any style `s`) is
            // carried verbatim, using the full qualified name so a namespaced
            // attribute survives, so a de-share reproduces the cell tag intact.
            let name = String::from_utf8_lossy(attr.key.as_ref());
            let value = String::from_utf8_lossy(&attr.value);
            attributes.push_str(&format!(r#" {name}="{value}""#));
        }
        let (row, column) = reference
            .as_deref()
            .and_then(parse_cell_ref)
            .ok_or_else(|| Error::invalid_xml("cell missing a valid `r` reference".to_owned()))?;
        Ok(Self {
            row,
            column,
            cell_type,
            start,
            attributes,
            text_open: None,
            inline_text: None,
            value_open: None,
            value_text: None,
        })
    }

    /// Finish the cell at its `</c>` end offset, yielding a [`SheetCell`] when it
    /// carries redactable text (a resolvable shared index or inline text).
    fn finish(self, end: usize) -> Result<Option<SheetCell>> {
        match self.cell_type {
            CellType::Shared => {
                let Some(index) = self
                    .value_text
                    .as_deref()
                    .and_then(|v| v.trim().parse::<usize>().ok())
                else {
                    // A shared cell with no parseable index is malformed but not
                    // fatal — it simply names no string to redact.
                    return Ok(None);
                };
                Ok(Some(SheetCell {
                    row: self.row,
                    column: self.column,
                    shared_index: Some(index),
                    inline_text: None,
                    source: CellSource::Shared {
                        index,
                        cell: self.start..end,
                        attributes: self.attributes,
                    },
                }))
            }
            CellType::Inline => {
                let Some(text) = self.inline_text else {
                    return Ok(None);
                };
                Ok(Some(SheetCell {
                    row: self.row,
                    column: self.column,
                    shared_index: None,
                    inline_text: Some(text.clone()),
                    source: CellSource::Inline { text },
                }))
            }
            CellType::Other => Ok(None),
        }
    }
}

/// The byte length of a leading UTF-8 BOM (`U+FEFF`), or 0 if absent.
fn bom_len(raw: &str) -> usize {
    if raw.starts_with('\u{feff}') { 3 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData>"#,
        r#"<row r="1"><c r="A1" t="s"><v>0</v></c></row>"#,
        r#"<row r="2"><c r="A2" t="s"><v>1</v></c>"#,
        r#"<c r="B2" t="inlineStr"><is><t>123-45-6789</t></is></c></row>"#,
        r#"<row r="3"><c r="C3"><v>42</v></c></row>"#,
        r#"</sheetData></worksheet>"#,
    );

    #[test]
    fn extracts_shared_and_inline_cells_skips_numbers() {
        let cells = parse_cells(SHEET).unwrap();
        assert_eq!(cells.len(), 3, "two shared + one inline, number skipped");

        // A1 shared index 0.
        assert_eq!((cells[0].row, cells[0].column), (0, 0));
        assert_eq!(cells[0].shared_index, Some(0));

        // A2 shared index 1.
        assert_eq!((cells[1].row, cells[1].column), (1, 0));
        assert_eq!(cells[1].shared_index, Some(1));

        // B2 inline: its text span slices exactly the SSN.
        assert_eq!((cells[2].row, cells[2].column), (1, 1));
        let text = cells[2].inline_text.clone().unwrap();
        assert_eq!(&SHEET[text], "123-45-6789");
    }

    #[test]
    fn shared_cell_span_covers_the_whole_element() {
        let cells = parse_cells(SHEET).unwrap();
        let CellSource::Shared { cell, .. } = &cells[0].source else {
            panic!("A1 is shared");
        };
        assert_eq!(&SHEET[cell.clone()], r#"<c r="A1" t="s"><v>0</v></c>"#);
    }

    #[test]
    fn preserves_non_type_attributes_for_de_share() {
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData>"#,
            r#"<row r="1"><c r="A1" s="3" t="s"><v>0</v></c></row>"#,
            r#"</sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        let CellSource::Shared { attributes, .. } = &cells[0].source else {
            panic!("shared");
        };
        assert!(attributes.contains(r#"r="A1""#));
        assert!(attributes.contains(r#"s="3""#));
        assert!(
            !attributes.contains("t="),
            "the type attribute is not carried"
        );
    }

    #[test]
    fn handles_a_leading_bom() {
        let sheet = format!("\u{feff}{SHEET}");
        let cells = parse_cells(&sheet).unwrap();
        let text = cells[2].inline_text.clone().unwrap();
        assert_eq!(&sheet[text], "123-45-6789");
    }

    #[test]
    fn a_self_closing_cell_yields_nothing() {
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData>"#,
            r#"<row r="1"><c r="A1"/></row></sheetData></worksheet>"#,
        );
        assert!(parse_cells(sheet).unwrap().is_empty());
    }
}
