//! End-to-end txt codec suite. Scenarios live under `format_txt/`; add
//! one by dropping a file there and a `mod` line in `format_txt/mod.rs`.

#[path = "support/mod.rs"]
mod support;

#[path = "format_txt/mod.rs"]
mod format_txt;
