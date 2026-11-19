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
mod names;
mod sanitize;
mod sheet;
mod strings;
mod workbook;

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use bytes::Bytes;
use elide_core::{Error, Result};
use quick_xml::escape::escape;

use self::sheet::parse_cells;
use self::strings::{parse_shared_strings, shared_string_items};
use self::workbook::{Sheet, resolve_sheets};
use crate::opc::index::text;
use crate::opc::xml_edit::{splice, splice_checked};
use crate::opc::{
    Extraction, Names, Package, PartClassifier, PartKind, PartPath, PartReplacement, PartRole,
    Replacement, TextAttribute,
};

/// The well-known part path of the workbook and its shared-string table.
const WORKBOOK_PART: &str = "xl/workbook.xml";
const WORKBOOK_RELS_PART: &str = "xl/_rels/workbook.xml.rels";
const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

/// The XLSX part classifier: maps each kind of part to the neutral
/// [`PartRole`] the engine acts on.
///
/// The worksheets and the shared-string table are cell parts, redacted through
/// the cell model, their text outside the cells through the shared text path;
/// the workbook (its defined names), comments (kept only when collaboration
/// content is redacted), drawings, and charts hold element text and
/// relationships parts external hyperlink targets, redacted through the shared
/// text path; an embedding is binary; the document properties are a property
/// part; everything else is structure. Sheet and table column names, data
/// validation messages, and the other [text attributes](sanitize::TEXT_ATTRIBUTES)
/// are redacted too.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SheetClassifier;

impl PartClassifier for SheetClassifier {
    fn role(&self, kind: PartKind) -> PartRole {
        match kind {
            PartKind::Story => PartRole::Cells,
            PartKind::Main
            | PartKind::Comments
            | PartKind::Chart
            | PartKind::Diagram
            | PartKind::Drawing => PartRole::ElementText,
            PartKind::Relationships => PartRole::RelationshipTargets,
            PartKind::Embedding(kind) => PartRole::Binary(kind),
            PartKind::Properties => PartRole::Property,
            _ => PartRole::Structure,
        }
    }

    fn text_attributes(&self) -> &'static [TextAttribute] {
        sanitize::TEXT_ATTRIBUTES
    }

    fn names(&self) -> &'static Names {
        &names::NAMES
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
    /// - [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
    ///   bytes are not a zip;
    /// - [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
    ///   workbook part is missing;
    /// - [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the workbook
    ///   or its relationships are malformed.
    pub fn open(document: &[u8]) -> Result<Self> {
        let package = Package::open(document, &SheetClassifier)?;
        let workbook = package
            .part_bytes(WORKBOOK_PART)
            .ok_or_else(|| Error::malformed("missing workbook part `xl/workbook.xml`"))?;
        let workbook = text(WORKBOOK_PART, &workbook)?;
        // A workbook with no relationships part has no resolvable sheets; treat
        // its rels as empty rather than failing the open.
        let rels = package.part_bytes(WORKBOOK_RELS_PART);
        let rels = rels
            .as_deref()
            .map(|b| text(WORKBOOK_RELS_PART, b))
            .transpose()?
            .unwrap_or_default();
        let sheets = resolve_sheets(workbook, rels)?;
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
    /// - [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
    ///   referenced worksheet part is missing;
    /// - [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a sheet or the
    ///   shared-string table is not UTF-8 or is malformed.
    pub fn extract(&self) -> Result<Vec<Cell>> {
        let shared = self.shared_strings()?;
        let mut cells = Vec::new();
        for sheet in &self.sheets {
            let bytes = self
                .package
                .part_bytes(sheet.part.as_str())
                .ok_or_else(|| {
                    Error::malformed(format!(
                        "worksheet `{}` referenced by sheet `{}` is missing",
                        sheet.part, sheet.name
                    ))
                })?;
            for cell in parse_cells(text(&sheet.part, &bytes)?)? {
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

    /// Rewrite cell `edits`, splice `text` replacements into the workbook's text
    /// outside its cells (the defined names, comments, drawings, charts,
    /// relationship targets, text attributes, and the cell parts' own text
    /// outside their cells), and replace whole embedding and document-property
    /// `parts`, re-packing every other part byte-for-byte.
    ///
    /// A replacement in a cell part is spliced before the cell edits land on
    /// it, so the two never overlap; one in a sheet, defined, table, or
    /// table-column name replaces the whole name, fitted to Excel's rules for
    /// it (see [`names`]).
    ///
    /// A cell edit naming an unknown sheet, a text replacement outside a
    /// text-bearing part, or a part replacement naming anything but an
    /// embedding or a property part, is refused fail-closed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`](elide_core::ErrorKind::Processing) if an edit or replacement cannot be applied, or
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a sheet cannot be read.
    pub fn rewrite(
        &self,
        edits: &[CellEdit],
        text: &[Replacement],
        parts: &[PartReplacement],
    ) -> Result<Vec<u8>> {
        let text = self.package.fit_names(text)?;

        // Splice the replacements that land in a cell part now, so the cell
        // edits apply on top of them.
        let mut in_cells: HashMap<&PartPath, Vec<&Replacement>> = HashMap::new();
        let mut elsewhere = Vec::new();
        for replacement in &text {
            if self.package.role_of(&replacement.part) == Some(PartRole::Cells) {
                in_cells
                    .entry(&replacement.part)
                    .or_default()
                    .push(replacement);
            } else {
                elsewhere.push(replacement.clone());
            }
        }
        let mut spliced: HashMap<PartPath, String> = HashMap::new();
        for (part, replacements) in in_cells {
            spliced.insert(part.clone(), self.package.splice_part(part, &replacements)?);
        }

        // Group edits by the sheet part they land in, keyed by (row, column).
        let mut by_part: HashMap<&PartPath, HashMap<(u32, u32), &CellEdit>> = HashMap::new();
        for edit in edits {
            let sheet = self
                .sheets
                .iter()
                .find(|s| s.name == edit.sheet.as_str())
                .ok_or_else(|| {
                    Error::processing(format!("edit names unknown sheet `{}`", edit.sheet))
                })?;
            by_part
                .entry(&sheet.part)
                .or_default()
                .insert((edit.row, edit.column), edit);
        }

        // Splice each affected sheet into new bytes, and hand them to the engine
        // as whole-part replacements for the byte-faithful re-zip.
        let mut replacements = Vec::new();
        for (part, cell_edits) in &by_part {
            let raw = self.current_text(&spliced, part)?;
            let edited = self.splice_sheet(part.as_str(), &raw, cell_edits)?;
            replacements.push(PartReplacement::new((*part).clone(), edited));
        }

        // De-sharing a `t="s"` cell drops a reference to its pooled string; a
        // pool entry that loses its last reference would otherwise linger in
        // `sharedStrings.xml` as orphaned bytes, a leak. Blank any such entry.
        let pruned = self.prune_orphaned_strings(&by_part, &spliced)?;
        let strings_pruned = pruned.is_some();
        replacements.extend(pruned);

        // A cell part spliced but otherwise unedited goes back as spliced.
        for (part, text) in &spliced {
            let rewritten = by_part.contains_key(part)
                || (part.as_str() == SHARED_STRINGS_PART && strings_pruned);
            if !rewritten {
                replacements.push(PartReplacement::new(part.clone(), text.clone()));
            }
        }

        // Fold in the redacted embeddings and document-property parts. A cell
        // part is rewritten only through cell edits, so a caller's replacement
        // of one (or of any other part) is refused.
        for part in parts {
            let role = self.package.role_of(&part.part);
            if !matches!(role, Some(PartRole::Property | PartRole::Binary(_))) {
                return Err(Error::processing(format!(
                    "part replacement targets non-redactable part `{}`",
                    part.part
                )));
            }
            replacements.push(part.clone());
        }

        self.package.rewrite_fitted(&elsewhere, &replacements)
    }

    /// The workbook's content outside its cells: the text blocks of its
    /// comments, drawings, and charts and the external hyperlink targets of its
    /// relationships parts, its binary embeddings, and any part that could not
    /// be read.
    pub fn markup(&self) -> Extraction {
        self.package.extract()
    }

    /// A cell part's text: as `spliced` holds it, or as the package holds it.
    fn current_text<'a>(
        &self,
        spliced: &'a HashMap<PartPath, String>,
        part: &PartPath,
    ) -> Result<Cow<'a, str>> {
        if let Some(text) = spliced.get(part) {
            return Ok(Cow::Borrowed(text));
        }
        let bytes = self
            .package
            .part_bytes(part.as_str())
            .ok_or_else(|| Error::processing(format!("part `{part}` not found")))?;
        text(part, &bytes).map(|text| Cow::Owned(text.to_owned()))
    }

    /// Blank every shared-string entry that, after the de-share edits in
    /// `by_part` over the cell parts' current text (`spliced`, or as the
    /// package holds it), no `t="s"` cell
    /// references any longer, so an orphaned pooled
    /// value cannot survive in the output. Returns the rewritten
    /// `sharedStrings.xml` part, or `None` if nothing is orphaned (or the
    /// workbook has no shared-string table).
    ///
    /// An orphaned entry is blanked to an empty `<si><t></t></si>` in place, so
    /// every other entry keeps its index and the still-referenced strings are
    /// untouched.
    fn prune_orphaned_strings(
        &self,
        by_part: &HashMap<&PartPath, HashMap<(u32, u32), &CellEdit>>,
        spliced: &HashMap<PartPath, String>,
    ) -> Result<Option<PartReplacement>> {
        if !self.package.contains_part(SHARED_STRINGS_PART) {
            return Ok(None);
        }
        let shared = PartPath::from(SHARED_STRINGS_PART);
        let raw = self.current_text(spliced, &shared)?;
        let items = shared_string_items(&raw)?;

        // Mark, across every sheet, each pool index a `t="s"` cell still
        // references after the edits: a shared cell keeps its reference unless an
        // edit de-shares it (an inline cell never referenced the pool).
        let mut referenced = vec![false; items.len()];
        for path in self.sheets.iter().map(|s| &s.part) {
            if !self.package.contains_part(path.as_str()) {
                continue;
            }
            let sheet_raw = self.current_text(spliced, path)?;
            let edited = by_part.get(path);
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

        // Blank each now-unreferenced, non-empty entry, its index kept.
        let orphans: Vec<(Range<usize>, &str)> = items
            .iter()
            .zip(&referenced)
            .filter(|(item, keep)| !**keep && !item.text.is_empty())
            .map(|(item, _)| (item.inner.clone(), "<t></t>"))
            .collect();
        if orphans.is_empty() {
            return Ok(None);
        }
        Ok(Some(PartReplacement::new(shared, splice(&raw, orphans))))
    }

    /// The shared-string table, or an empty table if the workbook has none.
    fn shared_strings(&self) -> Result<Vec<String>> {
        match self.package.part_bytes(SHARED_STRINGS_PART) {
            Some(bytes) => parse_shared_strings(text(SHARED_STRINGS_PART, &bytes)?),
            None => Ok(Vec::new()),
        }
    }

    /// Splice `edits` into one sheet's `raw` bytes: inline cells in place, shared
    /// cells de-shared. Overlapping cells are rejected.
    fn splice_sheet(
        &self,
        part: &str,
        raw: &str,
        edits: &HashMap<(u32, u32), &CellEdit>,
    ) -> Result<String> {
        // Resolve each edited cell to a splice (a byte range + its new bytes),
        // then apply them in order of where they start.
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
            let attributes = &cell.attributes;
            let replacement = format!(r#"<c{attributes} t="inlineStr"><is><t>{safe}</t></is></c>"#);
            splices.push((cell.cell.clone(), replacement));
        }

        // Fail closed if any edit did not land on a text-bearing cell: a redaction
        // that produced no splice would otherwise leave its target untouched.
        if matched != edits.len() {
            return Err(Error::processing(format!(
                "in `{part}`, {} of {} cell edits matched no text-bearing cell",
                edits.len() - matched,
                edits.len()
            )));
        }

        // Reject overlapping splices (two edits landing on the same/nested span)
        // and ones off the sheet's character boundaries.
        splice_checked(raw, part, splices)
    }
}

#[cfg(test)]
mod tests;
