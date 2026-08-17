//! End-to-end suite for the OOXML office formats (DOCX, XLSX, …).
//!
//! They all sit on the shared OPC package engine, so they share one test
//! binary and one set of package helpers. Each format is a submodule of
//! [`format_ooxml`] with its own scenarios; add a format by dropping a
//! folder under `format_ooxml/` and a `pub mod` line in its `mod.rs`.

#[path = "support/mod.rs"]
mod support;

#[path = "format_ooxml/mod.rs"]
mod format_ooxml;
