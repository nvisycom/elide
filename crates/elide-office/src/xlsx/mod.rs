//! [`Xlsx`]: an opened XLSX workbook, its cells extracted and redacted in place
//! over the shared [`opc`](crate::opc) engine.
//!
//! A workbook's user text lives in its worksheet cells, either as shared
//! strings (`xl/sharedStrings.xml`, referenced by index) or inline strings (in
//! the sheet itself). This facade reads those cells, addressed by sheet, row,
//! and column, and rewrites them byte-faithfully. Redacting a shared-string cell
//! *de-shares* it: the cell becomes an inline string carrying the redacted text,
//! so the pooled value other cells still reference is left untouched.

mod coord;
mod names;
mod sanitize;
mod sheet;
mod strings;
mod workbook;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use elide_core::{Error, Result};
use hipstr::HipStr;
use quick_xml::escape::escape;

pub(crate) use self::coord::CellCoord;
use self::sheet::{CellValue, parse_cells};
use self::strings::{parse_shared_strings, shared_string_items};
use self::workbook::{Sheet, resolve_sheets};
use crate::opc::package::index::text;
use crate::opc::xml::edit::{Cut, splice, splice_checked};
use crate::opc::{
    Extraction, Names, Package, PartClassifier, PartKind, PartPath, PartReplacement, PartRole,
    Replacement, TextAttribute, attribute_value,
};

/// The well-known part path of the workbook.
const WORKBOOK_PART: &str = "xl/workbook.xml";

/// The well-known part path of the workbook's relationships, which map each
/// sheet's relationship id to its part.
const WORKBOOK_RELS_PART: &str = "xl/_rels/workbook.xml.rels";

/// The well-known part path of the workbook's shared-string table.
const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

/// The XLSX format seam: maps each kind of part to the neutral [`PartRole`]
/// the engine acts on.
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
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SheetFormat;

impl PartClassifier for SheetFormat {
    fn role(&self, kind: PartKind) -> PartRole {
        match kind {
            PartKind::Story => PartRole::Cells,
            PartKind::Main => PartRole::ElementText,
            kind => PartRole::common(kind),
        }
    }

    fn text_attributes(&self) -> &'static [TextAttribute] {
        sanitize::TEXT_ATTRIBUTES
    }

    fn names(&self) -> &'static Names {
        &names::NAMES
    }
}

/// The entity-decoded value of a raw attribute value, for comparison or
/// parsing.
///
/// An attribute's raw value is its on-the-wire text, so one written with
/// character references, e.g. `t="inline&#83;tr"`, would not compare equal to
/// `inlineStr`. Decoding resolves those references so a cell's type and
/// reference, and a sheet's name and relationship id, are read correctly
/// regardless of how they were escaped. The raw value is still used where an
/// attribute is re-emitted verbatim.
fn decoded_attribute(raw: &str) -> Result<String> {
    attribute_value(raw).map(Cow::into_owned)
}

/// One text-bearing cell of the workbook: its sheet, its zero-based cell
/// coordinates, and its decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    /// The display name of the sheet the cell is on.
    pub(crate) sheet: HipStr<'static>,
    /// The cell's zero-based row and column.
    pub(crate) coord: CellCoord,
    /// The cell's decoded text (shared or inline), as a reader sees it.
    pub(crate) text: String,
}

/// One cell edit: overwrite the cell at `coord` on `sheet` with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellEdit {
    /// The display name of the sheet holding the cell.
    pub(crate) sheet: HipStr<'static>,
    /// The cell's zero-based row and column.
    pub(crate) coord: CellCoord,
    /// The replacement text.
    pub(crate) text: String,
}

impl CellEdit {
    /// An edit setting the cell at `coord` on `sheet` to `text`.
    pub(crate) fn new(
        sheet: impl Into<HipStr<'static>>,
        coord: CellCoord,
        text: impl Into<String>,
    ) -> Self {
        Self {
            sheet: sheet.into(),
            coord,
            text: text.into(),
        }
    }
}

impl From<&Cell> for CellEdit {
    /// An edit setting `cell` to its current text.
    fn from(cell: &Cell) -> Self {
        Self::new(cell.sheet.clone(), cell.coord, cell.text.clone())
    }
}

/// The cell edits landing in each sheet part, keyed by the cell they edit.
type EditsByPart<'a> = HashMap<&'a PartPath, HashMap<CellCoord, &'a CellEdit>>;

/// An opened XLSX workbook: its parts read once, ready to read its
/// [`cells`](Xlsx::cells) and [`extract`](Xlsx::extract) its content outside
/// them, or [`rewrite`](Xlsx::rewrite) them back to bytes.
#[derive(Debug, Clone)]
pub(crate) struct Xlsx {
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
    pub(crate) fn open(document: &[u8]) -> Result<Self> {
        let package = Package::open(document, &SheetFormat)?;
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
        let sheets = resolve_sheets(workbook, rels, |part| {
            package.content_type(part).map(|ct| ct.as_str())
        })?;
        Ok(Self { package, sheets })
    }

    /// Every text-bearing cell of the workbook, addressed by sheet name and
    /// zero-based cell coordinates.
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
    pub(crate) fn cells(&self) -> Result<Vec<Cell>> {
        let shared = self.shared_strings()?;
        let mut cells = Vec::new();
        for sheet in &self.sheets {
            let bytes = self.package.part_bytes(&sheet.part).ok_or_else(|| {
                Error::malformed(format!(
                    "worksheet `{}` referenced by sheet `{}` is missing",
                    sheet.part, sheet.name
                ))
            })?;
            for cell in parse_cells(text(&sheet.part, &bytes)?)? {
                let text = match cell.value {
                    CellValue::Shared(index) => match shared.get(index) {
                        Some(text) => text.clone(),
                        // A dangling index names no string; nothing to redact.
                        None => continue,
                    },
                    // Inline and formula cells carry their already-decoded text.
                    CellValue::Inline(text) => text,
                };
                cells.push(Cell {
                    sheet: sheet.name.clone(),
                    coord: cell.coord,
                    text,
                });
            }
        }
        Ok(cells)
    }

    /// The workbook's content outside its cells: the text blocks of its
    /// comments, drawings, and charts and the external hyperlink targets of its
    /// relationships parts, its binary embeddings, and any part that could not
    /// be read.
    pub(crate) fn extract(&self) -> Extraction {
        self.package.extract()
    }

    /// The raw bytes of the part at `path`, or `None` if the workbook has no
    /// such part. For a caller parsing a property part (`docProps/*`) before
    /// deciding what to redact, then feeding edited bytes back through
    /// [`rewrite`](Xlsx::rewrite).
    pub(crate) fn part_bytes(&self, path: &str) -> Option<Bytes> {
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
    pub(crate) fn rewrite(
        &self,
        edits: &[CellEdit],
        text: &[Replacement],
        parts: &[PartReplacement],
    ) -> Result<Vec<u8>> {
        let (in_cells, elsewhere): (Vec<_>, Vec<_>) = self
            .package
            .fit_names(text)?
            .into_iter()
            .partition(|replacement| {
                self.package.role_of(&replacement.part) == Some(PartRole::Cells)
            });

        // Splice the replacements that land in a cell part now, so the cell
        // edits apply on top of them, then hand every rewritten cell part to the
        // engine as a whole-part replacement for the byte-faithful re-zip.
        let spliced = self.splice_cell_parts(&in_cells)?;
        let by_part = self.edits_by_part(edits)?;
        let mut replacements: Vec<PartReplacement> = self
            .rewrite_cell_parts(spliced, &by_part)?
            .into_iter()
            .map(|(part, text)| PartReplacement::new(part, text))
            .collect();

        // Fold in the redacted embeddings and document-property parts. A cell
        // part is rewritten only through cell edits, so a caller's replacement
        // of one (or of any other part) is refused.
        self.refuse_unredactable(parts)?;
        replacements.extend_from_slice(parts);

        self.package.rewrite_fitted(&elsewhere, &replacements)
    }

    /// The text of each cell part `replacements` land in, with them spliced in.
    fn splice_cell_parts(&self, replacements: &[Replacement]) -> Result<HashMap<PartPath, String>> {
        let mut by_part: HashMap<&PartPath, Vec<&Replacement>> = HashMap::new();
        for replacement in replacements {
            by_part
                .entry(&replacement.part)
                .or_default()
                .push(replacement);
        }
        by_part
            .into_iter()
            .map(|(part, replacements)| {
                Ok((part.clone(), self.package.splice_part(part, &replacements)?))
            })
            .collect()
    }

    /// `edits` grouped by the sheet part they land in, keyed by cell.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`](elide_core::ErrorKind::Processing) if an edit
    /// names an unknown sheet.
    fn edits_by_part<'a>(&'a self, edits: &'a [CellEdit]) -> Result<EditsByPart<'a>> {
        let mut by_part: EditsByPart<'a> = HashMap::new();
        for edit in edits {
            let sheet = self
                .sheets
                .iter()
                .find(|s| s.name == edit.sheet)
                .ok_or_else(|| {
                    Error::processing(format!("edit names unknown sheet `{}`", edit.sheet))
                })?;
            by_part
                .entry(&sheet.part)
                .or_default()
                .insert(edit.coord, edit);
        }
        Ok(by_part)
    }

    /// The final text of every cell part the rewrite changes: each part as
    /// `spliced`, overwritten by its cell edits from `by_part` where it has
    /// any, and the shared-string table with its orphaned entries blanked.
    fn rewrite_cell_parts(
        &self,
        spliced: HashMap<PartPath, String>,
        by_part: &EditsByPart<'_>,
    ) -> Result<HashMap<PartPath, String>> {
        let mut edited = Vec::with_capacity(by_part.len());
        for (part, cell_edits) in by_part {
            let raw = self.current_text(&spliced, part)?;
            edited.push((
                (*part).clone(),
                splice_sheet(part.as_str(), &raw, cell_edits)?,
            ));
        }

        // De-sharing a `t="s"` cell drops a reference to its pooled string; a
        // pool entry that loses its last reference would otherwise linger in
        // `sharedStrings.xml` as orphaned bytes, a leak. Blank any such entry.
        let pruned = self.prune_orphaned_strings(by_part, &spliced)?;

        let mut rewritten = spliced;
        rewritten.extend(edited);
        rewritten.extend(pruned);
        Ok(rewritten)
    }

    /// Refuse any of `parts` that replaces anything but an embedding or a
    /// document-property part.
    fn refuse_unredactable(&self, parts: &[PartReplacement]) -> Result<()> {
        for part in parts {
            let role = self.package.role_of(&part.part);
            if !matches!(role, Some(PartRole::Property | PartRole::Binary(_))) {
                return Err(Error::processing(format!(
                    "part replacement targets non-redactable part `{}`",
                    part.part
                )));
            }
        }
        Ok(())
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
            .part_bytes(part)
            .ok_or_else(|| Error::processing(format!("part `{part}` not found")))?;
        text(part, &bytes).map(|text| Cow::Owned(text.to_owned()))
    }

    /// Blank every shared-string entry that, after the de-share edits in
    /// `by_part` over the cell parts' current text (`spliced`, or as the
    /// package holds it), no `t="s"` cell references any longer, so an
    /// orphaned pooled value cannot survive in the output. Returns the
    /// shared-string part and its rewritten text, or `None` if nothing is
    /// orphaned (or the workbook has no shared-string table).
    ///
    /// An orphaned entry is blanked to an empty `<si><t></t></si>` in place, so
    /// every other entry keeps its index and the still-referenced strings are
    /// untouched.
    fn prune_orphaned_strings(
        &self,
        by_part: &EditsByPart<'_>,
        spliced: &HashMap<PartPath, String>,
    ) -> Result<Option<(PartPath, String)>> {
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
            if !self.package.contains_part(path) {
                continue;
            }
            let sheet_raw = self.current_text(spliced, path)?;
            let edited = by_part.get(path);
            for cell in parse_cells(&sheet_raw)? {
                let Some(index) = cell.value.shared_index() else {
                    continue;
                };
                let redacted = edited.is_some_and(|e| e.contains_key(&cell.coord));
                if !redacted && index < referenced.len() {
                    referenced[index] = true;
                }
            }
        }

        // Blank each now-unreferenced, non-empty entry, its index kept.
        let orphans: Vec<Cut<&str>> = items
            .iter()
            .zip(&referenced)
            .filter(|(item, keep)| !**keep && !item.text.is_empty())
            .map(|(item, _)| Cut::new(item.inner.clone(), "<t></t>"))
            .collect();
        if orphans.is_empty() {
            return Ok(None);
        }
        let pruned = splice(&raw, orphans);
        Ok(Some((shared, pruned)))
    }

    /// The shared-string table, or an empty table if the workbook has none.
    fn shared_strings(&self) -> Result<Vec<String>> {
        match self.package.part_bytes(SHARED_STRINGS_PART) {
            Some(bytes) => parse_shared_strings(text(SHARED_STRINGS_PART, &bytes)?),
            None => Ok(Vec::new()),
        }
    }
}

/// Splice `edits` into one sheet's `raw` bytes: inline cells in place, shared
/// cells de-shared. Overlapping cells are rejected.
fn splice_sheet(part: &str, raw: &str, edits: &HashMap<CellCoord, &CellEdit>) -> Result<String> {
    // Resolve each edited cell to a splice (a byte range + its new bytes),
    // then apply them in order of where they start.
    let cells = parse_cells(raw)?;
    let mut splices: Vec<Cut> = Vec::new();
    // The edits that landed, by the cell each names: two cells at one
    // reference count once, so neither can stand in for an edit that missed.
    let mut matched = HashSet::new();
    for cell in &cells {
        let Some(edit) = edits.get(&cell.coord) else {
            continue;
        };
        matched.insert(cell.coord);
        let safe = escape(edit.text.as_str());
        // Every text-bearing cell becomes a single-run inline string carrying
        // the redacted text, rewriting the whole `<c>` element rather than one
        // `<t>` so that no run of a multi-run value is left behind. A shared
        // cell is thereby de-shared; a cached string-formula cell drops its
        // `<f>` too, so the formula cannot recompute the redacted value.
        let attributes = &cell.attributes;
        let replacement = format!(r#"<c{attributes} t="inlineStr"><is><t>{safe}</t></is></c>"#);
        splices.push(Cut::new(cell.span.clone(), replacement));
    }

    // Fail closed if any edit did not land on a text-bearing cell: a redaction
    // that produced no splice would otherwise leave its target untouched.
    if matched.len() != edits.len() {
        return Err(Error::processing(format!(
            "in `{part}`, {} of {} cell edits matched no text-bearing cell",
            edits.len() - matched.len(),
            edits.len()
        )));
    }

    // Reject overlapping splices (two edits landing on the same/nested span)
    // and ones off the sheet's character boundaries.
    splice_checked(raw, part, splices)
}

#[cfg(test)]
mod tests;
