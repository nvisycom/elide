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

use std::fmt::Write as _;
use std::ops::Range;

use elide_core::{Error, Result};
use quick_xml::events::Event;

use super::cellref::CellCoord;
use super::decoded_attribute;
use crate::opc::xml_edit::SpannedReader;

/// One text-bearing cell of a worksheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SheetCell {
    /// The cell's zero-based coordinates (from its `r` reference, or its
    /// position when the reference is omitted).
    pub(crate) coord: CellCoord,
    /// Where the cell's text is: in the shared-string table or in the cell.
    pub(crate) value: CellValue,
    /// Byte range of the whole `<c …>…</c>` element in the sheet's bytes.
    ///
    /// A rewrite replaces the whole element with a fresh inline string: a
    /// shared cell is de-shared, and an inline cell (which may hold several
    /// rich-text runs) is collapsed to one run so no run's text is left behind.
    pub(crate) span: Range<usize>,
    /// The cell's attributes other than `t` (e.g. `r`, `s`), verbatim, so a
    /// rewrite preserves them and only swaps the type and body.
    pub(crate) attributes: String,
}

/// The text a [`SheetCell`] carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CellValue {
    /// A `t="s"` cell: the index of its string in the shared-string table.
    Shared(usize),
    /// An inline or cached-formula cell: its own text, already decoded and
    /// concatenated across every run.
    Inline(String),
}

impl CellValue {
    /// The shared-string index of a [`Shared`](Self::Shared) value.
    pub(crate) fn shared_index(&self) -> Option<usize> {
        match self {
            Self::Shared(index) => Some(*index),
            Self::Inline(_) => None,
        }
    }

    /// The text of an [`Inline`](Self::Inline) value.
    #[cfg(test)]
    fn inline_text(&self) -> Option<&str> {
        match self {
            Self::Shared(_) => None,
            Self::Inline(text) => Some(text),
        }
    }
}

/// The text-bearing cells of a worksheet part `raw`, in document order.
///
/// quick-xml reports byte positions relative to the text after a leading BOM, so
/// every recorded span is shifted back onto the original bytes.
pub(crate) fn parse_cells(raw: &str) -> Result<Vec<SheetCell>> {
    let mut reader = SpannedReader::new(raw);
    let mut cells = Vec::new();
    let mut open: Option<OpenCell> = None;
    // Row/column tracking so a cell with an omitted `r` reference can be placed:
    // `r` is optional in SpreadsheetML, defaulting to the current row and the
    // column after the previous cell.
    let mut row: u32 = 0;
    let mut next_column: u32 = 0;

    loop {
        let (event, span) = reader
            .read_event()
            .map_err(|e| Error::malformed(format!("worksheet malformed: {e}")))?;

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
                let cell = OpenCell::parse(&e, span.start, CellCoord::new(row, next_column))?;
                next_column = cell.coord.column + 1;
                open = Some(cell);
            }
            // A self-closing `<c r=".." />` holds no value; advance the column.
            Event::Empty(e) if e.local_name().as_ref() == "c" => {
                let cell = OpenCell::parse(&e, span.start, CellCoord::new(row, next_column))?;
                next_column = cell.coord.column + 1;
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
            Event::Start(e) if e.local_name().as_ref() == "v" => {
                if let Some(cell) = open.as_mut() {
                    cell.value_open = Some(span.end);
                }
            }
            Event::End(e) if e.local_name().as_ref() == "v" => {
                // Captured by span, as a `<t>` run is: quick-xml reports an
                // entity as its own event, so no single text event holds the
                // whole value.
                if let Some(cell) = open.as_mut()
                    && let Some(value_start) = cell.value_open.take()
                    && value_start < span.start
                {
                    cell.value = Some(value_start..span.start);
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
        let attr = attr.map_err(|err| Error::malformed(format!("row attribute: {err}")))?;
        if attr.key.local_name().as_ref() == "r" {
            let text = decoded_attribute(&attr.value)?;
            let one_based: u32 = text
                .trim()
                .parse()
                .map_err(|_| Error::malformed(format!("row reference `{text}` is not a number")))?;
            return Ok(Some(one_based.saturating_sub(1)));
        }
    }
    Ok(None)
}

/// A cell being assembled as its child events stream in.
struct OpenCell {
    coord: CellCoord,
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
    /// The inner byte range of a non-empty `<v>`: a shared-string index for
    /// a `t="s"` cell, a cached string for a `t="str"` one.
    value: Option<Range<usize>>,
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
    /// omitted the cell is placed at `default` (the enclosing row and the
    /// column after the previous cell).
    fn parse(
        e: &quick_xml::events::BytesStart<'_>,
        start: usize,
        default: CellCoord,
    ) -> Result<Self> {
        let mut reference = None;
        let mut cell_type = CellType::Other;
        let mut attributes = String::new();
        for attr in e.attributes() {
            let attr = attr.map_err(|err| Error::malformed(format!("cell attribute: {err}")))?;
            let local = attr.key.local_name();
            if local.as_ref() == "t" {
                // Compare the decoded value: `t` may be written with character
                // references and must still classify the cell correctly.
                cell_type = match decoded_attribute(&attr.value)?.as_str() {
                    "s" => CellType::Shared,
                    "inlineStr" => CellType::Inline,
                    "str" => CellType::Formula,
                    _ => CellType::Other,
                };
                // The type attribute is not carried: the rewrite sets its own.
                continue;
            }
            if local.as_ref() == "r" {
                reference = Some(decoded_attribute(&attr.value)?);
            }
            // Every non-`t` attribute (including `r` and any style `s`) is carried
            // verbatim, using the full qualified name so a namespaced attribute
            // survives. The value keeps its existing entities but any literal `"`
            // is escaped, since the rewrite wraps values in double quotes and a
            // single-quoted source value may contain a bare `"`.
            let name = attr.key.as_ref();
            let value = escape_attr_value(attr.value.as_ref());
            // Writing to a `String` cannot fail.
            let _ = write!(attributes, r#" {name}="{value}""#);
        }
        // A cell may omit `r`; when present it must be a valid reference.
        let coord = match reference.as_deref() {
            Some(reference) => reference.parse().map_err(|_| {
                Error::malformed(format!("cell reference `{reference}` is malformed"))
            })?,
            None => default,
        };
        Ok(Self {
            coord,
            cell_type,
            start,
            attributes,
            text_open: None,
            text_runs: Vec::new(),
            value_open: None,
            value: None,
        })
    }

    /// Finish the cell at its `</c>` end offset, yielding a [`SheetCell`] when it
    /// carries redactable text (a resolvable shared index, inline text, or a
    /// cached formula string).
    fn finish(self, raw: &str, end: usize) -> Result<Option<SheetCell>> {
        let value = match self.cell_type {
            CellType::Shared => {
                let Some(index) = self
                    .value
                    .and_then(|value| raw[value].trim().parse::<usize>().ok())
                else {
                    // A shared cell with no parseable index is malformed but not
                    // fatal, it simply names no string to redact.
                    return Ok(None);
                };
                CellValue::Shared(index)
            }
            CellType::Inline => {
                if self.text_runs.is_empty() {
                    return Ok(None);
                }
                // Concatenate every run's decoded text: the value a reader sees.
                CellValue::Inline(self.decoded_runs(raw))
            }
            CellType::Formula => {
                // `t="str"` caches the string result of a formula in `<v>`.
                let Some(value) = self.value else {
                    return Ok(None);
                };
                CellValue::Inline(unescape_text(&raw[value]))
            }
            CellType::Other => return Ok(None),
        };
        Ok(Some(SheetCell {
            coord: self.coord,
            value,
            span: self.start..end,
            attributes: self.attributes,
        }))
    }

    /// The concatenated, entity-decoded text of every `<t>` run.
    fn decoded_runs(&self, raw: &str) -> String {
        self.text_runs
            .iter()
            .map(|run| unescape_text(&raw[run.clone()]))
            .collect()
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
        assert_eq!(cells[0].coord, CellCoord::new(0, 0));
        assert_eq!(cells[0].value.shared_index(), Some(0));

        // A2 shared index 1.
        assert_eq!(cells[1].coord, CellCoord::new(1, 0));
        assert_eq!(cells[1].value.shared_index(), Some(1));

        // B2 inline: its text is exactly the SSN.
        assert_eq!(cells[2].coord, CellCoord::new(1, 1));
        assert_eq!(cells[2].value.inline_text(), Some("123-45-6789"));
    }

    #[test]
    fn shared_cell_span_covers_the_whole_element() {
        let cells = parse_cells(SHEET).unwrap();
        assert!(cells[0].value.shared_index().is_some(), "A1 is shared");
        assert_eq!(
            &SHEET[cells[0].span.clone()],
            r#"<c r="A1" t="s"><v>0</v></c>"#
        );
    }

    #[test]
    fn preserves_non_type_attributes_for_de_share() {
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData>"#,
            r#"<row r="1"><c r="A1" s="3" t="s"><v>0</v></c></row>"#,
            r#"</sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        assert!(cells[0].value.shared_index().is_some(), "shared");
        let attributes = &cells[0].attributes;
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
        assert_eq!(cells[2].value.inline_text(), Some("123-45-6789"));
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
        assert_eq!(cells[0].value.inline_text(), Some("alice@example.com"));
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
        assert_eq!(cells[0].value.inline_text(), Some("bob@example.com"));
    }

    #[test]
    fn a_cached_formula_string_keeps_the_text_around_an_entity() {
        // quick-xml reports an entity as its own event, between two texts: the
        // whole value is read, not only the text after the last entity.
        let sheet = concat!(
            r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
            r#"<c r="A1" t="str"><f>B1</f><v>Smith &amp; Jones</v></c>"#,
            r#"</row></sheetData></worksheet>"#,
        );
        let cells = parse_cells(sheet).unwrap();
        assert_eq!(cells[0].value.inline_text(), Some("Smith & Jones"));
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
        assert_eq!(cells[0].coord, CellCoord::new(1, 0));
        assert_eq!(cells[1].coord, CellCoord::new(1, 1));
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
        assert_eq!(cells[0].value.inline_text(), Some("alice@example.com"));
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
        assert_eq!(cells[0].coord, CellCoord::new(0, 0));
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
        assert!(cells[0].value.shared_index().is_some(), "shared");
        let attributes = &cells[0].attributes;
        assert!(
            attributes.contains(r#"foo="a&quot;b""#),
            "attributes: {attributes}"
        );
    }
}
