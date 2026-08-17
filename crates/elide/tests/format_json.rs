//! End-to-end json codec suite. Scenarios live under `format_json/`; add
//! one by dropping a file there and a `mod` line in `format_json/mod.rs`.

#[path = "support/mod.rs"]
mod support;

#[path = "format_json/mod.rs"]
mod format_json;
