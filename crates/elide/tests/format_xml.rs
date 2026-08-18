//! End-to-end xml codec suite. Scenarios live under `format_xml/`; add
//! one by dropping a file there and a `mod` line in `format_xml/mod.rs`.

#[path = "support/mod.rs"]
mod support;

#[path = "format_xml/mod.rs"]
mod format_xml;
