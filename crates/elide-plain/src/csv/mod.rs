//! CSV: the handler that streams cells and redacts within them.
//!
//! Cells hold text, so the handler reuses [`TextData`] as the chunk payload
//! and [`TextReplacement`] as the replacement; only the location is tabular (a
//! `(row, column)` address with an optional intra-cell byte range). CSV parses
//! and rewrites rows directly.
//!
//! [`TextData`]: elide_core::modality::text::TextData
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

mod handler;
mod loader;

pub use self::handler::{format, format_with};
