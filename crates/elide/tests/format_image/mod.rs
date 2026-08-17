//! Image-format scenarios, one module per format. They share the single
//! image handler (decode → paint region → re-encode) and OCR enrichment;
//! shared helpers live here as the family grows.
#![allow(dead_code)]

mod png;
