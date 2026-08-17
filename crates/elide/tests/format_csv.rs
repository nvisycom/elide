//! End-to-end csv codec suite. Scenarios live under `format_csv/`; add
//! one by dropping a file there and a `mod` line in `format_csv/mod.rs`.

#[path = "support/mod.rs"]
mod support;

#[path = "format_csv/mod.rs"]
mod format_csv;
