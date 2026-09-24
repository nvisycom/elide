//! Tabular modality: the CSV handler that streams cells and redacts within
//! them.
//!
//! Cells hold text, so the handler reuses [`TextData`] as the chunk payload
//! and [`TextReplacement`] as the replacement; only the location is tabular (a
//! `(row, column)` address with an optional intra-cell byte range). CSV parses
//! and rewrites rows directly.
//!
//! [`TextData`]: elide_core::modality::text::TextData
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

#[cfg(feature = "csv")]
mod csv_handler;
#[cfg(feature = "csv")]
mod csv_loader;

// `*_format` is `pub` so the parent `handler` module re-exports it as the
// crate's public contract; the loader/handler pairs stay `pub(crate)`.
#[cfg(feature = "csv")]
pub use self::csv_handler::{format as csv_format, format_with as csv_format_with};
