//! [`XlsxState`]: the workbook's editable cell state, shared between the body
//! stream and the recombiner.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use elide_codec::string::RedactRange;
use elide_core::modality::tabular::TabularLocation;
use elide_core::modality::text::TextReplacement;
use elide_core::{Error, ErrorKind, Result};

use super::XlsxCell;
use crate::xlsx::CellEdit;

/// The workbook's editable cells, behind the shared [`XlsxState`] lock.
#[derive(Debug)]
struct XlsxInner {
    /// The workbook's text-bearing cells, in extraction order.
    cells: Vec<XlsxCell>,
    /// Indices of the cells a redaction actually changed, so the re-pack rewrites
    /// only those, leaving unedited shared-string cells shared.
    changed: BTreeSet<usize>,
}

impl XlsxInner {
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
}

/// The workbook's editable cell state, shared between the body
/// [`XlsxStream`](super::stream::XlsxStream) and the
/// [`XlsxRecombine`](super::recombine::XlsxRecombine) so a redaction on the
/// stream is visible when the recombiner re-packs. `Clone` shares the one state
/// (an `Arc` bump); the lock is held only inside these methods.
#[derive(Clone)]
pub(super) struct XlsxState(Arc<Mutex<XlsxInner>>);

impl XlsxState {
    /// Wrap the extracted `cells`.
    pub(super) fn new(cells: Vec<XlsxCell>) -> Self {
        Self(Arc::new(Mutex::new(XlsxInner {
            cells,
            changed: BTreeSet::new(),
        })))
    }

    /// The cell at `index` in extraction order, cloned.
    pub(super) fn cell(&self, index: usize) -> Option<XlsxCell> {
        self.0.lock().unwrap().cells.get(index).cloned()
    }

    /// The header text for `column` on `sheet`: the text of the cell at row 0 of
    /// that column, if any. Provides column context to the recognizer.
    pub(super) fn column_header(&self, sheet: &str, column: u32) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .cells
            .iter()
            .find(|c| c.sheet == sheet && c.row == 0 && c.column == column)
            .map(|c| c.text.clone())
    }

    /// The cell text at `location`, if a unique cell exists.
    pub(super) fn cell_at(&self, location: &TabularLocation) -> Result<Option<String>> {
        let inner = self.0.lock().unwrap();
        let sheet = location.sheet_name.as_deref();
        let Some(index) = inner.cell_index(sheet, location.row_index, location.column_index)?
        else {
            return Ok(None);
        };
        Ok(Some(inner.cells[index].text.clone()))
    }

    /// Apply one cell edit at `location`, replacing its intra-cell range (or the
    /// whole cell when no range is set) with the replacement text.
    ///
    /// Fail-closed: a redaction that matches no cell is an error, not a silent
    /// no-op, so a request can never appear to succeed without changing the
    /// intended cell.
    pub(super) fn redact_one(
        &self,
        location: &TabularLocation,
        replacement: &TextReplacement,
    ) -> Result<()> {
        let mut inner = self.0.lock().unwrap();
        let sheet = location.sheet_name.as_deref();
        let index = inner
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
        let cell = &mut inner.cells[index];
        let start = location.start_offset.unwrap_or(0);
        let end = location.end_offset.unwrap_or(cell.text.len());
        let value = replacement.value().unwrap_or_default();
        cell.text.redact_range(value, start..end)?;
        // Only a cell that was actually edited is sent for rewrite, so unchanged
        // shared-string cells are not needlessly de-shared.
        inner.changed.insert(index);
        Ok(())
    }

    /// The [`CellEdit`]s for every cell a redaction changed, for the re-pack.
    /// Unedited shared-string cells are left shared.
    pub(super) fn cell_edits(&self) -> Vec<CellEdit> {
        let inner = self.0.lock().unwrap();
        inner
            .changed
            .iter()
            .map(|&index| {
                let cell = &inner.cells[index];
                CellEdit::new(cell.sheet.clone(), cell.row, cell.column, cell.text.clone())
            })
            .collect()
    }
}
