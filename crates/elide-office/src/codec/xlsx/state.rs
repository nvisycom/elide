//! [`XlsxState`]: the workbook's editable cell state, owned by the body stream.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use elide_codec::string::RedactRange;
use elide_core::modality::tabular::TabularLocation;
use elide_core::modality::text::TextReplacement;
use elide_core::{Error, Result};
use hipstr::HipStr;

use crate::xlsx::{Cell, CellCoord, CellEdit};

/// The workbook's editable cell state, owned by the body
/// [`XlsxStream`](super::stream::XlsxStream): redactions edit it, and the
/// [`XlsxDocument`](super::document::XlsxDocument) reads the changed cells
/// back to re-pack.
#[derive(Debug)]
pub(super) struct XlsxState {
    /// The workbook's text-bearing cells, in extraction order.
    cells: Vec<Cell>,
    /// Indices of the cells a redaction actually changed, so the re-pack rewrites
    /// only those, leaving unedited shared-string cells shared.
    changed: BTreeSet<usize>,
}

impl XlsxState {
    /// Wrap the extracted `cells`.
    pub(super) fn new(cells: Vec<Cell>) -> Self {
        Self {
            cells,
            changed: BTreeSet::new(),
        }
    }

    /// The extracted cells, in extraction order.
    pub(super) fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The header text of every column, keyed by `(sheet, column)`: the text of
    /// the first cell at row 0 of that column, built once so each cell's lookup
    /// is cheap. Provides column context to the recognizer.
    pub(super) fn column_headers(&self) -> HashMap<(&str, u32), HipStr<'static>> {
        let mut headers = HashMap::new();
        for cell in self.cells.iter().filter(|c| c.coord.row == 0) {
            headers
                .entry((cell.sheet.as_str(), cell.coord.column))
                .or_insert_with(|| HipStr::from(cell.text.clone()));
        }
        headers
    }

    /// The cell text at `location`, if a unique cell exists.
    pub(super) fn cell_at(&self, location: &TabularLocation) -> Result<Option<&str>> {
        let index = self.cell_index(location.sheet_name.as_deref(), coord(location))?;
        Ok(index.map(|index| self.cells[index].text.as_str()))
    }

    /// Apply a batch of cell edits atomically: every edit's cell is resolved and
    /// its splice range validated *before* any cell is mutated, so an
    /// unresolvable location (or an out-of-bounds / mid-character splice) fails
    /// the whole batch without leaving a partial edit behind.
    ///
    /// Fail-closed: a redaction that matches no cell is an error, not a silent
    /// no-op, so a request can never appear to succeed without changing the
    /// intended cell.
    pub(super) fn redact_cells(
        &mut self,
        edits: &[(TabularLocation, TextReplacement)],
    ) -> Result<()> {
        // Splice every edit into a per-cell working copy first, so both the cell
        // lookup and the splice itself (`redact_range` errors on a mid-character
        // range) are validated for the whole batch before any cell changes. A
        // batch with a later invalid edit fails without leaving an earlier one
        // committed. Several edits may target the same cell, so each works from
        // the running copy, not the original.
        let mut working: BTreeMap<usize, String> = BTreeMap::new();
        for (location, replacement) in edits {
            let sheet = location.sheet_name.as_deref();
            let index = self.cell_index(sheet, coord(location))?.ok_or_else(|| {
                Error::malformed(format!(
                    "xlsx redaction targets no cell at (row {}, column {}) on {}",
                    location.row_index,
                    location.column_index,
                    on_sheet(sheet)
                ))
            })?;
            let text = working
                .entry(index)
                .or_insert_with(|| self.cells[index].text.clone());
            let span = location.span.clone().unwrap_or(0..text.len());
            text.redact_with(replacement, span)?;
        }

        // Every splice succeeded: commit each cell's final text. Only a cell that
        // was actually edited is sent for rewrite, so unchanged shared-string
        // cells are not needlessly de-shared.
        for (index, text) in working {
            self.cells[index].text = text;
            self.changed.insert(index);
        }
        Ok(())
    }

    /// The index of the unique cell at `coord`, on `sheet` when one is named.
    ///
    /// A location with no sheet name must still identify exactly one cell: if the
    /// same coordinates exist on more than one sheet, the match is ambiguous and
    /// this errs rather than silently editing the wrong sheet. `Ok(None)` means no
    /// cell matched.
    fn cell_index(&self, sheet: Option<&str>, coord: CellCoord) -> Result<Option<usize>> {
        let mut matches = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.coord == coord && sheet.is_none_or(|name| c.sheet == name));
        let first = matches.next().map(|(i, _)| i);
        if first.is_some() && matches.next().is_some() {
            let CellCoord { row, column } = coord;
            return Err(Error::malformed(match sheet {
                Some(name) => format!(
                    "xlsx redaction at (row {row}, column {column}) is ambiguous \
                     on sheet `{name}`"
                ),
                None => format!(
                    "xlsx redaction at (row {row}, column {column}) is ambiguous \
                     across sheets; a sheet name is required"
                ),
            }));
        }
        Ok(first)
    }

    /// The [`CellEdit`]s for every cell a redaction changed, for the re-pack.
    /// Unedited shared-string cells are left shared.
    pub(super) fn cell_edits(&self) -> Vec<CellEdit> {
        self.changed
            .iter()
            .map(|&index| CellEdit::from(&self.cells[index]))
            .collect()
    }
}

/// The cell coordinates `location` addresses.
fn coord(location: &TabularLocation) -> CellCoord {
    CellCoord::new(location.row_index, location.column_index)
}

/// The sheet a lookup is scoped to, for an error message.
fn on_sheet(sheet: Option<&str>) -> String {
    match sheet {
        Some(name) => format!("sheet `{name}`"),
        None => "any sheet".to_owned(),
    }
}
