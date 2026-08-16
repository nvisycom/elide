//! Tabular modality: spreadsheet/CSV handlers that stream cells and
//! redact within them.
//!
//! Cells hold text, so tabular handlers reuse [`TextData`] as the chunk
//! payload and [`TextReplacement`] as the replacement; only the location
//! is tabular (a `(row, column)` address, with a sheet name for a
//! multi-sheet workbook, and an optional intra-cell byte range). CSV parses
//! and rewrites rows directly; XLSX reads and redacts workbook cells over
//! the shared [`elide_office`] package engine.
//!
//! [`TextData`]: elide_core::modality::text::TextData
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

#[cfg(feature = "csv")]
mod csv_handler;
#[cfg(feature = "csv")]
mod csv_loader;
#[cfg(feature = "xlsx")]
mod xlsx_handler;
#[cfg(feature = "xlsx")]
mod xlsx_loader;

// `*_format` is `pub` so the parent `handler` module re-exports it as the
// crate's public contract; the loader/handler pairs stay `pub(crate)`.
#[cfg(feature = "csv")]
pub use self::csv_handler::format as csv_format;
#[cfg(feature = "xlsx")]
pub use self::xlsx_handler::format as xlsx_format;
