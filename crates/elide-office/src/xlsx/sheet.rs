//! Parsing a worksheet part into cells, each carrying the byte spans a
//! redaction needs.
//!
//! A cell is `<c r="A1" t="..">…</c>`. Three kinds hold user text: `t="s"` names
//! a shared string by the index in its `<v>`, `t="inlineStr"` carries the text
//! directly in `<is>…<t>..</t></is>` (across one or more rich-text runs), and
//! `t="str"` caches the string result of a formula in `<v>`. Every other cell
//! (numbers, booleans, errors) holds no free text and is skipped. Row and column
//! come from the `r` reference, or, when it is omitted, from the enclosing row
//! and the preceding cell, so a sparse or reference-less sheet is placed
//! correctly.

use std::ops::Range;

use quick_xml::events::Event;
use quick_xml::events::attributes::Attribute;
use quick_xml::{Reader, XmlVersion};

use crate::error::{Error, Result};
use crate::xlsx::cellref::parse_cell_ref;

/// The entity-decoded value of an attribute, for comparison or parsing.
///
/// `Attribute::value` is the raw on-the-wire bytes, so an attribute written with
/// character references, e.g. `t="inline&#83;tr"`, would not compare equal to
/// `inlineStr`. Normalizing resolves those references so a cell's type and
/// reference are read correctly regardless of how they were escaped. The raw
/// value is still used where an attribute is re-emitted verbatim.
fn normalized(attr: &Attribute<'_>) -> Result<String> {
    attr.normalized_value(XmlVersion::Implicit1_0)
        .map(|value| value.into_owned())
        .map_err(|e| Error::invalid_xml(format!("attribute value: {e}")))
}

/// Where a cell's text lives, and the byte spans to rewrite it.
///
/// Both variants rewrite by replacing the *whole* `<c>…</c>` element with a
/// fresh inline string: a shared cell is de-shared, and an inline cell (which may
/// hold several rich-text runs) is collapsed to one run so no run's text is left
/// behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CellSource {
    /// A shared string: the index into the shared-string table, plus the byte
    /// range of the whole `<c>…</c>` element.
    Shared {
        /// Index into the shared-string table.
        index: usize,
        /// Byte range of the whole `<c …>…</c>` element in the sheet's bytes.
        cell: Range<usize>,
        /// The cell's attributes other than `t` (e.g. `r`, `s`), verbatim, so a
        /// rewrite preserves them and only swaps the type and body.
        attributes: String,
    },
    /// An inline string (`t="inlineStr"`) or a cached string-formula result
    /// (`t="str"`): the byte range of the whole `<c>…</c>` element, replaced
    /// wholesale so every run of a multi-run value is redacted.
    Inline {
        /// Byte range of the whole `<c …>…</c>` element in the sheet's bytes.
        cell: Range<usize>,
        /// The cell's attributes other than `t`, verbatim.
        attributes: String,
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
    /// the text; `None` for an inline or formula cell.
    pub(crate) shared_index: Option<usize>,
    /// The cell's own text (inline or cached-formula), already concatenated
    /// across every run; `None` for a shared cell whose text is in the table.
    pub(crate) inline_text: Option<String>,
    /// The cell's byte span and attributes, retained for the rewrite.
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
    // Row/column tracking so a cell with an omitted `r` reference can be placed:
    // `r` is optional in SpreadsheetML, defaulting to the current row and the
    // column after the previous cell.
    let mut row: u32 = 0;
    let mut next_column: u32 = 0;

    loop {
        let event = reader.read_event().map_err(malformed)?;
        let span = last..reader.buffer_position() as usize + bom;
        last = span.end;

        match event {
            Event::Eof => break,
            Event::Start(e) if e.local_name().as_ref() == "row" => {
                row = row_index(&e)?.unwrap_or(row);
                next_column = 0;
            }
            Event::End(e) if e.local_name().as_ref() == "row" => {
                row = row.saturating_add(1);
            }
            Event::Start(e) if e.local_name().as_ref() == "c" => {
                let cell = OpenCell::parse(&e, span.start, row, next_column)?;
                next_column = cell.column + 1;
                open = Some(cell);
            }
            // A self-closing `<c r=".." />` holds no value; advance the column.
            Event::Empty(e) if e.local_name().as_ref() == "c" => {
                let cell = OpenCell::parse(&e, span.start, row, next_column)?;
                next_column = cell.column + 1;
                open = None;
            }
            Event::Start(e) if e.local_name().as_ref() == "t" => {
                if let Some(cell) = open.as_mut() {
                    cell.text_open = Some(span.end);
                }
            }
            Event::End(e) if e.local_name().as_ref() == "t" => {
                if let Some(cell) = open.as_mut()
                    && let Some(text_start) = cell.text_open.take()
                {
                    // Collect every `<t>` run of the cell, not just the last, so a
                    // rich-text inline string keeps all its text.
                    cell.text_runs.push(text_start..span.start);
                }
            }
            Event::Text(t) if open.as_ref().and_then(|c| c.value_open).is_some() => {
                // The `<v>` of a shared cell holds the shared-string index.
                if let Some(cell) = open.as_mut() {
                    cell.value_text = Some(t.as_ref().to_owned());
                }
            }
            Event::Start(e) if e.local_name().as_ref() == "v" => {
                if let Some(cell) = open.as_mut() {
                    cell.value_open = Some(span.end);
                }
            }
            Event::End(e) if e.local_name().as_ref() == "v" => {
                if let Some(cell) = open.as_mut() {
                    cell.value_open = None;
                }
            }
            Event::End(e) if e.local_name().as_ref() == "c" => {
                if let Some(cell) = open.take()
                    && let Some(finished) = cell.finish(raw, span.end)?
                {
                    cells.push(finished);
                }
            }
            _ => {}
        }
    }
    Ok(cells)
}

/// The zero-based row index from a `<row r="N">` start tag, if present (`r` is
/// one-based in the file). `None` when the row has no `r`.
fn row_index(e: &quick_xml::events::BytesStart<'_>) -> Result<Option<u32>> {
    for attr in e.attributes() {
        let attr = attr.map_err(|err| Error::invalid_xml(format!("row attribute: {err}")))?;
        if attr.key.local_name().as_ref() == "r" {
            let text = normalized(&attr)?;
            let one_based: u32 = text.trim().parse().map_err(|_| {
                Error::invalid_xml(format!("row reference `{text}` is not a number"))
            })?;
            return Ok(Some(one_based.saturating_sub(1)));
        }
    }
    Ok(None)
}

/// A cell being assembled as its child events stream in.
struct OpenCell {
    row: u32,
    column: u32,
    cell_type: CellType,
    /// Byte offset where the `<c …>` element begins.
    start: usize,
    /// Attributes other than `t`, verbatim (values `"`-escaped), for the rewrite.
    attributes: String,
    /// Byte offset just past a `<t>` open, while inside it.
    text_open: Option<usize>,
    /// The inner byte range of every `<t>` run of an inline/formula cell, in
    /// order, so a multi-run rich-text value keeps all its text.
    text_runs: Vec<Range<usize>>,
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
    /// `t="inlineStr"`: text in `<is>…<t>`.
    Inline,
    /// `t="str"`: the cached text result of a string formula, in `<v>`.
    Formula,
    /// Anything else (number, boolean, error, …): no free text.
    Other,
}

impl OpenCell {
    /// Read a `<c>` start tag's `r`, `t`, and other attributes. When `r` is
    /// omitted the cell is placed at `default_row` / `default_column` (the
    /// enclosing row and the column after the previous cell).
    fn parse(
        e: &quick_xml::events::BytesStart<'_>,
        start: usize,
        default_row: u32,
        default_column: u32,
    ) -> Result<Self> {
        let mut reference = None;
        let mut cell_type = CellType::Other;
        let mut attributes = String::new();
        for attr in e.attributes() {
            let attr = attr.map_err(|err| Error::invalid_xml(format!("cell attribute: {err}")))?;
            let local = attr.key.local_name();
            if local.as_ref() == "t" {
                // Compare the decoded value: `t` may be written with character
                // references and must still classify the cell correctly.
                cell_type = match normalized(&attr)?.as_str() {
                    "s" => CellType::Shared,
                    "inlineStr" => CellType::Inline,
                    "str" => CellType::Formula,
                    _ => CellType::Other,
                };
                // The type attribute is not carried: the rewrite sets its own.
                continue;
            }
            if local.as_ref() == "r" {
                reference = Some(normalized(&attr)?);
            }
            // Every non-`t` attribute (including `r` and any style `s`) is carried
            // verbatim, using the full qualified name so a namespaced attribute
            // survives. The value keeps its existing entities but any literal `"`
            // is escaped, since the rewrite wraps values in double quotes and a
            // single-quoted source value may contain a bare `"`.
            let name = attr.key.as_ref();
            let value = escape_attr_value(attr.value.as_ref());
            attributes.push_str(&format!(r#" {name}="{value}""#));
        }
        // A cell may omit `r`; when present it must be a valid reference.
        let (row, column) = match reference.as_deref() {
            Some(reference) => parse_cell_ref(reference).ok_or_else(|| {
                Error::invalid_xml(format!("cell reference `{reference}` is malformed"))
            })?,
            None => (default_row, default_column),
        };
        Ok(Self {
            row,
            column,
            cell_type,
            start,
            attributes,
            text_open: None,
            text_runs: Vec::new(),
            value_open: None,
            value_text: None,
        })
    }

    /// Finish the cell at its `</c>` end offset, yielding a [`SheetCell`] when it
    /// carries redactable text (a resolvable shared index, inline text, or a
    /// cached formula string).
    fn finish(self, raw: &str, end: usize) -> Result<Option<SheetCell>> {
        match self.cell_type {
            CellType::Shared => {
                let Some(index) = self
                    .value_text
                    .as_deref()
                    .and_then(|v| v.trim().parse::<usize>().ok())
                else {
                    // A shared cell with no parseable index is malformed but not
                    // fatal, it simply names no string to redact.
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
                if self.text_runs.is_empty() {
                    return Ok(None);
                }
                // Concatenate every run's decoded text: the value a reader sees.
                let text = self.decoded_runs(raw)?;
                Ok(Some(SheetCell {
                    row: self.row,
                    column: self.column,
                    shared_index: None,
                    inline_text: Some(text),
                    source: CellSource::Inline {
                        cell: self.start..end,
                        attributes: self.attributes,
                    },
                }))
            }
            CellType::Formula => {
                // `t="str"` caches the string result of a formula in `<v>`.
                let Some(text) = self.value_text.clone() else {
                    return Ok(None);
                };
                Ok(Some(SheetCell {
                    row: self.row,
                    column: self.column,
                    shared_index: None,
                    inline_text: Some(unescape_text(&text)),
                    source: CellSource::Inline {
                        cell: self.start..end,
                        attributes: self.attributes,
                    },
                }))
            }
            CellType::Other => Ok(None),
        }
    }

    /// The concatenated, entity-decoded text of every `<t>` run.
    fn decoded_runs(&self, raw: &str) -> Result<String> {
        let mut text = String::new();
        for run in &self.text_runs {
            text.push_str(&unescape_text(&raw[run.clone()]));
        }
        Ok(text)
    }
}

/// Decode XML entities in raw cell text.
fn unescape_text(text: &str) -> String {
    match quick_xml::escape::unescape(text) {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => text.to_owned(),
    }
}

/// Escape a literal `"` in an attribute value as `&quot;`, leaving existing
/// entities (`&amp;`, `&#..;`) untouched, so the value can be re-wrapped in
/// double quotes without breaking the tag or double-escaping.
fn escape_attr_value(value: &str) -> String {
    value.replace('"', "&quot;")
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

        // B2 inline: its text is exactly the SSN.
        assert_eq!((cells[2].row, cells[2].column), (1, 1));
        assert_eq!(cells[2].inline_text.as_deref(), Some("123-45-6789"));
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
        assert_eq!(cells[2].inline_text.as_deref(), Some("123-45-6789"));
    }

    #[test]
    fn a_self_closing_cell_yields_nothing() {
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData>"#,
            r#"<row r="1"><c r="A1"/></row></sheetData></worksheet>"#,
        );
        assert!(parse_cells(sheet).unwrap().is_empty());
    }

    #[test]
    fn a_multi_run_inline_string_keeps_every_run() {
        // A rich-text inline value split across runs: all runs must be collected,
        // not just the last, or the earlier text leaks unredacted.
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
            r#"<c r="A1" t="inlineStr"><is><r><t>alice@</t></r><r><t>example.com</t></r></is></c>"#,
            r#"</row></sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        assert_eq!(cells[0].inline_text.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn a_cached_formula_string_is_text_bearing() {
        // `t="str"` holds a formula's cached string result, user-visible text.
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
            r#"<c r="A1" t="str"><f>B1</f><v>bob@example.com</v></c>"#,
            r#"</row></sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].inline_text.as_deref(), Some("bob@example.com"));
    }

    #[test]
    fn a_cell_without_a_reference_is_placed_by_position() {
        // `r` is optional: the first cell defaults to column 0 of its row, the
        // next to column 1, and so on.
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData><row r="2">"#,
            r#"<c t="inlineStr"><is><t>x</t></is></c>"#,
            r#"<c t="inlineStr"><is><t>y</t></is></c>"#,
            r#"</row></sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        assert_eq!((cells[0].row, cells[0].column), (1, 0));
        assert_eq!((cells[1].row, cells[1].column), (1, 1));
    }

    #[test]
    fn an_entity_encoded_type_attribute_is_still_classified() {
        // `t="inline&#83;tr"` decodes to `inlineStr`; comparing the raw value
        // would misclassify the cell as non-text and leak its content.
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
            r#"<c r="A1" t="inline&#83;tr"><is><t>alice@example.com</t></is></c>"#,
            r#"</row></sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        assert_eq!(
            cells.len(),
            1,
            "the cell must be recognized as text-bearing"
        );
        assert_eq!(cells[0].inline_text.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn an_entity_encoded_cell_reference_is_parsed() {
        // A `&#65;` in the reference decodes to `A`, so `&#65;1` is `A1`.
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
            r#"<c r="&#65;1" t="inlineStr"><is><t>x</t></is></c>"#,
            r#"</row></sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        assert_eq!((cells[0].row, cells[0].column), (0, 0));
    }

    #[test]
    fn a_single_quoted_attribute_value_is_quote_escaped() {
        // A source attribute may be single-quoted and contain a literal `"`;
        // carrying it verbatim into a double-quoted rewrite would break the tag.
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
            r##"<c r="A1" foo='a"b' t="s"><v>0</v></c>"##,
            r#"</row></sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        let CellSource::Shared { attributes, .. } = &cells[0].source else {
            panic!("shared");
        };
        assert!(
            attributes.contains(r#"foo="a&quot;b""#),
            "attributes: {attributes}"
        );
    }
}
