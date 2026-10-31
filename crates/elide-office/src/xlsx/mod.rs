//! [`Xlsx`]: an opened XLSX workbook, its cells extracted and redacted in place
//! over the shared [`opc`](crate::opc) engine.
//!
//! A workbook's user text lives in its worksheet cells, either as shared
//! strings (`xl/sharedStrings.xml`, referenced by index) or inline strings (in
//! the sheet itself). This facade reads those cells, addressed by sheet, row,
//! and column, and rewrites them byte-faithfully. Redacting a shared-string cell
//! *de-shares* it: the cell becomes an inline string carrying the redacted text,
//! so the pooled value other cells still reference is left untouched.

mod cellref;
mod sanitize;
mod sheet;
mod strings;
mod workbook;

use std::collections::HashMap;
use std::ops::Range;

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use quick_xml::escape::escape;

use self::sheet::{CellSource, parse_cells};
use self::strings::{parse_shared_strings, shared_string_items};
use self::workbook::{Sheet, resolve_sheets};
use crate::opc::{
    Extraction, Package, PartClassifier, PartKind, PartPath, PartReplacement, PartRole, Replacement,
};

/// The well-known part path of the workbook and its shared-string table.
const WORKBOOK_PART: &str = "xl/workbook.xml";
const WORKBOOK_RELS_PART: &str = "xl/_rels/workbook.xml.rels";
const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

/// The XLSX part classifier: maps each kind of part to the neutral
/// [`PartRole`] the engine acts on.
///
/// The worksheets and the shared-string table are cell parts, redacted through
/// the cell model; comments (kept only when collaboration content is
/// redacted), drawings, and charts hold element text and relationships parts
/// external hyperlink targets, redacted through the shared text path; an
/// embedding is binary; the document properties are a property part; the
/// workbook and everything else is structure.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SheetClassifier;

impl PartClassifier for SheetClassifier {
    fn role(&self, kind: PartKind) -> PartRole {
        match kind {
            PartKind::Story => PartRole::Cells,
            PartKind::Comments | PartKind::Chart | PartKind::Diagram | PartKind::Drawing => {
                PartRole::ElementText
            }
            PartKind::Relationships => PartRole::RelationshipTargets,
            PartKind::Embedding(kind) => PartRole::Binary(kind),
            PartKind::Properties => PartRole::Property,
            _ => PartRole::Structure,
        }
    }
}

/// One text-bearing cell of the workbook: its sheet, its zero-based cell
/// coordinates, and its decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// The display name of the sheet the cell is on.
    pub sheet: String,
    /// Zero-based row index.
    pub row: u32,
    /// Zero-based column index.
    pub column: u32,
    /// The cell's decoded text (shared or inline), as a reader sees it.
    pub text: String,
}

/// One cell edit: overwrite the cell at `(sheet, row, column)` with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellEdit {
    /// The display name of the sheet holding the cell.
    pub sheet: String,
    /// Zero-based row index.
    pub row: u32,
    /// Zero-based column index.
    pub column: u32,
    /// The replacement text.
    pub text: String,
}

impl CellEdit {
    /// An edit setting the cell at `(sheet, row, column)` to `text`.
    pub fn new(sheet: impl Into<String>, row: u32, column: u32, text: impl Into<String>) -> Self {
        Self {
            sheet: sheet.into(),
            row,
            column,
            text: text.into(),
        }
    }
}

/// An opened XLSX workbook: its parts read once, ready to
/// [`extract`](Xlsx::extract) its cells or [`rewrite`](Xlsx::rewrite) them back
/// to bytes.
#[derive(Debug, Clone)]
pub struct Xlsx {
    package: Package,
    /// The workbook's sheets, in tab order: display name and part path.
    sheets: Vec<Sheet>,
}

impl Xlsx {
    /// Open an XLSX from its bytes, reading every part and resolving its sheets.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MalformedInput`] if the
    ///   bytes are not a zip;
    /// - [`ErrorKind::MalformedInput`] if the
    ///   workbook part is missing;
    /// - [`ErrorKind::MalformedInput`] if the workbook
    ///   or its relationships are malformed.
    pub fn open(document: &[u8]) -> Result<Self> {
        let package = Package::open(document, &SheetClassifier)?;
        if !package.contains_part(WORKBOOK_PART) {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                "missing workbook part `xl/workbook.xml`",
            ));
        }
        let workbook = part_text(&package, WORKBOOK_PART)?;
        // A workbook with no relationships part has no resolvable sheets; treat
        // its rels as empty rather than failing the open.
        let rels = package
            .part_bytes(WORKBOOK_RELS_PART)
            .map(|b| decode_part(WORKBOOK_RELS_PART, &b))
            .transpose()?
            .unwrap_or_default();
        let sheets = resolve_sheets(&workbook, &rels)?;
        Ok(Self { package, sheets })
    }

    /// Extract every text-bearing cell of the workbook, addressed by sheet name
    /// and zero-based cell coordinates.
    ///
    /// A shared-string cell resolves its text through the shared-string table; an
    /// inline-string or cached-formula-string cell carries its text directly.
    /// Numeric, boolean, and other typed cells hold no free text and are skipped.
    ///
    /// **Fail-closed:** a sheet the workbook references but does not contain is an
    /// error, not a silently-skipped part, so a workbook can never report a
    /// successful extraction while an unread worksheet's text survives.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MalformedInput`] if a
    ///   referenced worksheet part is missing;
    /// - [`ErrorKind::MalformedInput`] if a sheet or the
    ///   shared-string table is not UTF-8 or is malformed.
    pub fn extract(&self) -> Result<Vec<Cell>> {
        let shared = self.shared_strings()?;
        let mut cells = Vec::new();
        for sheet in &self.sheets {
            let bytes = self.package.part_bytes(&sheet.part).ok_or_else(|| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!(
                        "worksheet `{}` referenced by sheet `{}` is missing",
                        sheet.part, sheet.name
                    ),
                )
            })?;
            let raw = decode_part(&sheet.part, &bytes)?;
            for cell in parse_cells(&raw)? {
                let text = match (cell.shared_index, cell.inline_text) {
                    (Some(index), _) => match shared.get(index) {
                        Some(text) => text.clone(),
                        // A dangling index names no string; nothing to redact.
                        None => continue,
                    },
                    // Inline and formula cells carry their already-decoded text.
                    (None, Some(text)) => text,
                    (None, None) => continue,
                };
                cells.push(Cell {
                    sheet: sheet.name.clone(),
                    row: cell.row,
                    column: cell.column,
                    text,
                });
            }
        }
        Ok(cells)
    }

    /// The raw bytes of the part at `path`, or `None` if the workbook has no
    /// such part. For a caller parsing a property part (`docProps/*`) before
    /// deciding what to redact, then feeding edited bytes back through
    /// [`rewrite`](Xlsx::rewrite).
    pub fn part_bytes(&self, path: &str) -> Option<Bytes> {
        self.package.part_bytes(path)
    }

    /// Rewrite cell `edits`, splice `text` replacements into the non-cell text
    /// parts (comments, drawings, charts, relationship targets), and replace
    /// whole embedding and document-property `parts`, re-packing every other
    /// part byte-for-byte.
    ///
    /// A cell edit naming an unknown sheet, a text replacement outside a
    /// text-bearing part, or a part replacement naming anything but an
    /// embedding or a property part, is refused fail-closed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if an edit or replacement cannot be applied, or
    /// [`ErrorKind::MalformedInput`] if a sheet cannot be read.
    pub fn rewrite(
        &self,
        edits: &[CellEdit],
        text: &[Replacement],
        parts: &[PartReplacement],
    ) -> Result<Vec<u8>> {
        // Group edits by the sheet part they land in, keyed by (row, column).
        let mut by_part: HashMap<String, HashMap<(u32, u32), &CellEdit>> = HashMap::new();
        for edit in edits {
            let sheet = self
                .sheets
                .iter()
                .find(|s| s.name == edit.sheet.as_str())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::Processing,
                        format!("edit names unknown sheet `{}`", edit.sheet),
                    )
                })?;
            by_part
                .entry(sheet.part.clone())
                .or_default()
                .insert((edit.row, edit.column), edit);
        }

        // Splice each affected sheet into new bytes, and hand them to the engine
        // as whole-part replacements for the byte-faithful re-zip.
        let mut replacements = Vec::new();
        for (part, cell_edits) in &by_part {
            let bytes = self.package.part_bytes(part).ok_or_else(|| {
                Error::new(
                    ErrorKind::Processing,
                    format!("sheet part `{part}` not found"),
                )
            })?;
            let raw = decode_part(part, &bytes)?;
            let spliced = self.splice_sheet(part, &raw, cell_edits)?;
            replacements.push(PartReplacement::new(
                PartPath::from(part.clone()),
                spliced.into_bytes(),
            ));
        }

        // De-sharing a `t="s"` cell drops a reference to its pooled string; a
        // pool entry that loses its last reference would otherwise linger in
        // `sharedStrings.xml` as orphaned bytes, a leak. Blank any such entry.
        if let Some(pruned) = self.prune_orphaned_strings(&by_part)? {
            replacements.push(pruned);
        }

        // Fold in the redacted embeddings and document-property parts. A cell
        // part is rewritten only through cell edits, so a caller's replacement
        // of one (or of any other part) is refused.
        for part in parts {
            let role = self.package.role_of(&part.part);
            if !matches!(role, Some(PartRole::Property | PartRole::Binary(_))) {
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!(
                        "part replacement targets non-redactable part `{}`",
                        part.part
                    ),
                ));
            }
            replacements.push(part.clone());
        }

        self.package.rewrite_with_parts(text, &replacements)
    }

    /// The workbook's content outside its cells: the text blocks of its
    /// comments, drawings, and charts and the external hyperlink targets of its
    /// relationships parts, its binary embeddings, and any part that could not
    /// be read.
    pub fn markup(&self) -> Extraction {
        self.package.extract()
    }

    /// Blank every shared-string entry that, after the de-share edits in
    /// `by_part`, no `t="s"` cell references any longer, so an orphaned pooled
    /// value cannot survive in the output. Returns the rewritten
    /// `sharedStrings.xml` part, or `None` if nothing is orphaned (or the
    /// workbook has no shared-string table).
    ///
    /// An orphaned entry is blanked to an empty `<si><t></t></si>` in place, so
    /// every other entry keeps its index and the still-referenced strings are
    /// untouched.
    fn prune_orphaned_strings(
        &self,
        by_part: &HashMap<String, HashMap<(u32, u32), &CellEdit>>,
    ) -> Result<Option<PartReplacement>> {
        let Some(bytes) = self.package.part_bytes(SHARED_STRINGS_PART) else {
            return Ok(None);
        };
        let raw = decode_part(SHARED_STRINGS_PART, &bytes)?;
        let items = shared_string_items(&raw)?;

        // Count, across every sheet, the `t="s"` cells that still reference each
        // pool index after the edits: a shared cell keeps its reference unless an
        // edit de-shares it (an inline cell never referenced the pool).
        let mut referenced = vec![false; items.len()];
        for path in self.sheets.iter().map(|s| &s.part) {
            let Some(sheet_bytes) = self.package.part_bytes(path) else {
                continue;
            };
            let sheet_raw = decode_part(path, &sheet_bytes)?;
            let edited = by_part.get(path.as_str());
            for cell in parse_cells(&sheet_raw)? {
                let Some(index) = cell.shared_index else {
                    continue;
                };
                let redacted = edited.is_some_and(|e| e.contains_key(&(cell.row, cell.column)));
                if !redacted && index < referenced.len() {
                    referenced[index] = true;
                }
            }
        }

        // Blank each now-unreferenced, non-empty entry, highest offset first so
        // earlier spans stay valid.
        let mut orphans: Vec<&Range<usize>> = items
            .iter()
            .zip(&referenced)
            .filter(|(item, keep)| !**keep && !item.text.is_empty())
            .map(|(item, _)| &item.inner)
            .collect();
        if orphans.is_empty() {
            return Ok(None);
        }
        orphans.sort_by_key(|range| range.start);

        let mut out = String::with_capacity(raw.len());
        let mut cursor = 0usize;
        for inner in orphans {
            out.push_str(&raw[cursor..inner.start]);
            out.push_str("<t></t>");
            cursor = inner.end;
        }
        out.push_str(&raw[cursor..]);
        Ok(Some(PartReplacement::new(
            PartPath::from(SHARED_STRINGS_PART),
            out.into_bytes(),
        )))
    }

    /// The shared-string table, or an empty table if the workbook has none.
    fn shared_strings(&self) -> Result<Vec<String>> {
        match self.package.part_bytes(SHARED_STRINGS_PART) {
            Some(bytes) => parse_shared_strings(&decode_part(SHARED_STRINGS_PART, &bytes)?),
            None => Ok(Vec::new()),
        }
    }

    /// Splice `edits` into one sheet's `raw` bytes: inline cells in place, shared
    /// cells de-shared. Edits are applied right-to-left so each edit's length
    /// delta leaves earlier spans valid; overlapping cells are rejected.
    fn splice_sheet(
        &self,
        part: &str,
        raw: &str,
        edits: &HashMap<(u32, u32), &CellEdit>,
    ) -> Result<String> {
        // Resolve each edited cell to a splice (a byte range + its new bytes),
        // then apply from the highest offset down.
        let cells = parse_cells(raw)?;
        let mut splices: Vec<(Range<usize>, String)> = Vec::new();
        let mut matched = 0usize;
        for cell in &cells {
            let Some(edit) = edits.get(&(cell.row, cell.column)) else {
                continue;
            };
            matched += 1;
            let safe = escape(edit.text.as_str());
            // Every text-bearing cell becomes a single-run inline string carrying
            // the redacted text, rewriting the whole `<c>` element rather than one
            // `<t>` so that no run of a multi-run value is left behind. A shared
            // cell is thereby de-shared; a cached string-formula cell drops its
            // `<f>` too, so the formula cannot recompute the redacted value.
            let (range, attributes) = match &cell.source {
                CellSource::Shared {
                    cell, attributes, ..
                }
                | CellSource::Inline {
                    cell, attributes, ..
                } => (cell.clone(), attributes),
            };
            let replacement = format!(r#"<c{attributes} t="inlineStr"><is><t>{safe}</t></is></c>"#);
            splices.push((range, replacement));
        }

        // Fail closed if any edit did not land on a text-bearing cell: a redaction
        // that produced no splice would otherwise leave its target untouched.
        if matched != edits.len() {
            return Err(Error::new(
                ErrorKind::Processing,
                format!(
                    "in `{part}`, {} of {} cell edits matched no text-bearing cell",
                    edits.len() - matched,
                    edits.len()
                ),
            ));
        }

        // Reject overlapping splices (two edits landing on the same/nested span)
        // and apply highest-first so byte offsets stay valid as the string grows
        // or shrinks.
        splices.sort_by_key(|(range, _)| range.start);
        let mut prev_end = 0usize;
        for (range, _) in &splices {
            if range.start < prev_end {
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!("overlapping cell edits in `{part}`"),
                ));
            }
            if range.end > raw.len()
                || !raw.is_char_boundary(range.start)
                || !raw.is_char_boundary(range.end)
            {
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!(
                        "cell edit span {}..{} out of bounds in `{part}`",
                        range.start, range.end
                    ),
                ));
            }
            prev_end = range.end;
        }

        let mut out = String::with_capacity(raw.len());
        let mut cursor = 0usize;
        for (range, text) in splices {
            out.push_str(&raw[cursor..range.start]);
            out.push_str(&text);
            cursor = range.end;
        }
        out.push_str(&raw[cursor..]);
        Ok(out)
    }
}

/// Read a required part's bytes as UTF-8 text.
fn part_text(package: &Package, path: &str) -> Result<String> {
    let bytes = package
        .part_bytes(path)
        .ok_or_else(|| Error::new(ErrorKind::MalformedInput, format!("missing part `{path}`")))?;
    decode_part(path, &bytes)
}

/// Decode a part's bytes as UTF-8, or an [`ErrorKind::MalformedInput`] naming it.
///
/// [`ErrorKind::MalformedInput`]: elide_core::ErrorKind::MalformedInput
fn decode_part(path: &str, bytes: &[u8]) -> Result<String> {
    std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("part `{path}` is not UTF-8"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Excel-authored workbook (Microsoft): one sheet `in`, a header row
    /// and two data rows of PII (names, emails, phones, cards, IBANs, SSNs, IPs)
    /// stored as shared strings, with a full styles/theme package.
    const SAMPLE: &[u8] = include_bytes!("../../tests/testdata/sample.xlsx");

    /// A hand-built workbook whose shared string `alice@example.com` (index 1) is
    /// referenced by both `Customers!A2` and `Notes!A1`, plus an inline SSN at
    /// `Customers!B2`. Its two sheets sharing a pooled value drive the de-share
    /// and orphan-prune tests, which a single-sheet workbook cannot exercise.
    const SHARED: &[u8] = include_bytes!("../../tests/testdata/shared_across_sheets.xlsx");

    fn cell<'a>(cells: &'a [Cell], sheet: &str, row: u32, col: u32) -> Option<&'a Cell> {
        cells
            .iter()
            .find(|c| c.sheet == sheet && c.row == row && c.column == col)
    }

    #[test]
    fn opens_and_resolves_a_real_workbook() {
        let xlsx = Xlsx::open(SAMPLE).expect("opens");
        assert_eq!(xlsx.sheets.len(), 1);
        assert_eq!(xlsx.sheets[0].name, "in");
    }

    #[test]
    fn classifier_maps_kinds_to_roles() {
        assert_eq!(
            SheetClassifier.role(PartKind::Properties),
            PartRole::Property
        );
        // A worksheet is a cell part, a structure part stays structure.
        assert_eq!(SheetClassifier.role(PartKind::Story), PartRole::Cells);
        assert_eq!(
            SheetClassifier.role(PartKind::Structure),
            PartRole::Structure
        );
        assert_eq!(
            SheetClassifier.role(PartKind::Comments),
            PartRole::ElementText
        );
    }

    #[test]
    fn open_rejects_a_non_workbook_zip() {
        // A valid zip lacking xl/workbook.xml is not a usable workbook.
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            use zip::write::SimpleFileOptions;
            let mut zip = zip::ZipWriter::new(&mut cursor);
            zip.start_file("random.txt", SimpleFileOptions::default())
                .unwrap();
            use std::io::Write;
            zip.write_all(b"hi").unwrap();
            zip.finish().unwrap();
        }
        assert!(Xlsx::open(&cursor.into_inner()).is_err());
    }

    #[test]
    fn extracts_shared_string_cells_from_a_real_workbook() {
        let cells = Xlsx::open(SAMPLE).unwrap().extract().unwrap();
        // The header labels and the first data row's PII, on sheet `in`.
        assert_eq!(cell(&cells, "in", 0, 1).unwrap().text, "email");
        assert_eq!(
            cell(&cells, "in", 1, 1).unwrap().text,
            "alice.johnson@example.com"
        );
        assert_eq!(cell(&cells, "in", 1, 5).unwrap().text, "123-45-6789");
        assert_eq!(
            cell(&cells, "in", 2, 1).unwrap().text,
            "bob.smith@example.com"
        );
    }

    #[test]
    fn extracts_shared_and_inline_cells() {
        let cells = Xlsx::open(SHARED).unwrap().extract().unwrap();
        assert_eq!(cell(&cells, "Customers", 0, 0).unwrap().text, "Email");
        assert_eq!(
            cell(&cells, "Customers", 1, 0).unwrap().text,
            "alice@example.com"
        );
        assert_eq!(
            cell(&cells, "Customers", 2, 0).unwrap().text,
            "bob@example.com"
        );
        // Inline SSN.
        assert_eq!(cell(&cells, "Customers", 1, 1).unwrap().text, "123-45-6789");
        // The shared value reused on the second sheet.
        assert_eq!(
            cell(&cells, "Notes", 0, 0).unwrap().text,
            "alice@example.com"
        );
    }

    #[test]
    fn de_share_inlines_one_cell_and_leaves_the_pool_and_other_sheet() {
        let xlsx = Xlsx::open(SHARED).unwrap();
        // Redact only sheet1!A2 (alice). sheet2!A1 shares the same pool entry.
        let edit = CellEdit {
            sheet: String::from("Customers"),
            row: 1,
            column: 0,
            text: String::from("[EMAIL]"),
        };
        let out = xlsx.rewrite(&[edit], &[], &[]).unwrap();

        // Re-open the output and read it back.
        let cells = Xlsx::open(&out).unwrap().extract().unwrap();
        assert_eq!(cell(&cells, "Customers", 1, 0).unwrap().text, "[EMAIL]");
        // The OTHER sheet's cell, sharing the pool entry, is untouched.
        assert_eq!(
            cell(&cells, "Notes", 0, 0).unwrap().text,
            "alice@example.com"
        );
        // The pooled value survives in the shared-string part.
        let pool = String::from_utf8(read_part(&out, SHARED_STRINGS_PART)).unwrap();
        assert!(pool.contains("alice@example.com"), "pool: {pool}");
        // The edited cell is now an inline string.
        let sheet1 = String::from_utf8(read_part(&out, "xl/worksheets/sheet1.xml")).unwrap();
        assert!(
            sheet1.contains(r#"<c r="A2" t="inlineStr"><is><t>[EMAIL]</t></is></c>"#),
            "sheet1: {sheet1}"
        );
    }

    #[test]
    fn redacting_every_reference_prunes_the_orphaned_pool_entry() {
        let xlsx = Xlsx::open(SHARED).unwrap();
        // Redact BOTH cells that reference alice's pool entry (sheet1!A2 and
        // sheet2!A1). With no reference left, the pooled value must not survive.
        let edits = [
            CellEdit {
                sheet: String::from("Customers"),
                row: 1,
                column: 0,
                text: String::from("[EMAIL]"),
            },
            CellEdit {
                sheet: String::from("Notes"),
                row: 0,
                column: 0,
                text: String::from("[EMAIL]"),
            },
        ];
        let out = xlsx.rewrite(&edits, &[], &[]).unwrap();

        // The orphaned pool entry is blanked; alice is gone from every part.
        let pool = String::from_utf8(read_part(&out, SHARED_STRINGS_PART)).unwrap();
        assert!(
            !pool.contains("alice@example.com"),
            "orphan survived: {pool}"
        );
        // bob is still referenced (sheet1!A3), so its entry is untouched.
        assert!(
            pool.contains("bob@example.com"),
            "pruned a live entry: {pool}"
        );
        // Whole-file: no alice bytes anywhere in the output.
        for part in [
            "xl/sharedStrings.xml",
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
        ] {
            let bytes = read_part(&out, part);
            assert!(
                !String::from_utf8_lossy(&bytes).contains("alice@example.com"),
                "alice leaked in {part}"
            );
        }
    }

    #[test]
    fn inline_edit_splices_in_place() {
        let xlsx = Xlsx::open(SHARED).unwrap();
        let edit = CellEdit {
            sheet: String::from("Customers"),
            row: 1,
            column: 1,
            text: String::from("[SSN]"),
        };
        let out = xlsx.rewrite(&[edit], &[], &[]).unwrap();
        let sheet1 = String::from_utf8(read_part(&out, "xl/worksheets/sheet1.xml")).unwrap();
        assert!(sheet1.contains("<t>[SSN]</t>"), "sheet1: {sheet1}");
        assert!(!sheet1.contains("123-45-6789"));
    }

    #[test]
    fn edit_text_is_xml_escaped() {
        let xlsx = Xlsx::open(SHARED).unwrap();
        let edit = CellEdit {
            sheet: String::from("Customers"),
            row: 1,
            column: 1,
            text: String::from("a & b <c>"),
        };
        let out = xlsx.rewrite(&[edit], &[], &[]).unwrap();
        let cells = Xlsx::open(&out).unwrap().extract().unwrap();
        // Round-trips through escaping back to the literal text.
        assert_eq!(cell(&cells, "Customers", 1, 1).unwrap().text, "a & b <c>");
    }

    #[test]
    fn rewrite_rejects_an_unknown_sheet() {
        let xlsx = Xlsx::open(SHARED).unwrap();
        let edit = CellEdit {
            sheet: String::from("Ghost"),
            row: 0,
            column: 0,
            text: String::from("x"),
        };
        assert!(xlsx.rewrite(&[edit], &[], &[]).is_err());
    }

    #[test]
    fn unedited_parts_are_byte_identical() {
        let xlsx = Xlsx::open(SHARED).unwrap();
        let edit = CellEdit {
            sheet: String::from("Customers"),
            row: 1,
            column: 1,
            text: String::from("[SSN]"),
        };
        let out = xlsx.rewrite(&[edit], &[], &[]).unwrap();
        // sheet2 was not edited, so its bytes are unchanged.
        assert_eq!(
            read_part(&out, "xl/worksheets/sheet2.xml"),
            read_part(SHARED, "xl/worksheets/sheet2.xml"),
        );
    }

    /// A second workbook carrying non-cell text: a cell comment
    /// (`xl/comments1.xml`) and a drawing (`xl/drawings/drawing1.xml`), each with
    /// PII.
    const SAMPLE2: &[u8] = include_bytes!("../../tests/testdata/sample2.xlsx");

    #[test]
    fn markup_covers_comments_drawings_and_relationship_targets() {
        let markup = Xlsx::open(SAMPLE2).unwrap().markup();
        assert!(markup.issues.is_empty(), "{:?}", markup.issues);
        let parts: Vec<&str> = markup.blocks.iter().map(|b| b.part.as_str()).collect();
        assert!(
            parts.contains(&"xl/comments1.xml"),
            "comment part missing: {parts:?}"
        );
        assert!(
            parts.contains(&"xl/drawings/drawing1.xml"),
            "drawing part missing: {parts:?}"
        );
        // External hyperlink targets carry PII redacted as attribute values.
        assert!(
            parts.contains(&"xl/worksheets/_rels/sheet1.xml.rels"),
            "sheet relationships part missing: {parts:?}"
        );
        // The worksheet itself (its cells) is redacted through the cell model.
        assert!(!parts.contains(&"xl/worksheets/sheet1.xml"));
    }

    #[test]
    fn a_text_replacement_splices_byte_faithfully() {
        let xlsx = Xlsx::open(SAMPLE2).unwrap();
        let markup = xlsx.markup();
        let block = markup
            .blocks
            .iter()
            .find(|b| b.part.as_str() == "xl/comments1.xml" && b.text.contains("carol@example.com"))
            .expect("comment block");
        let out = xlsx
            .rewrite(&[], &[Replacement::for_block(block, "[EMAIL]")], &[])
            .unwrap();
        let comment = String::from_utf8(read_part(&out, "xl/comments1.xml")).unwrap();
        assert!(comment.contains("[EMAIL]") && !comment.contains("carol@example.com"));
        // Another part is untouched.
        assert_eq!(
            read_part(&out, "xl/drawings/drawing1.xml"),
            read_part(SAMPLE2, "xl/drawings/drawing1.xml"),
        );
    }

    #[test]
    fn a_part_replacement_naming_a_cell_part_is_refused() {
        let xlsx = Xlsx::open(SAMPLE2).unwrap();
        // A worksheet is a cell part, redacted through cell edits only.
        let replacement =
            PartReplacement::new(PartPath::from("xl/worksheets/sheet1.xml"), b"<x/>".to_vec());
        assert!(xlsx.rewrite(&[], &[], &[replacement]).is_err());
    }

    /// Read one part's bytes out of an XLSX zip.
    fn read_part(bytes: &[u8], name: &str) -> Vec<u8> {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        let mut entry = zip.by_name(name).unwrap();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        buf
    }
}
