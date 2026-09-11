//! Image-format scenarios, named `<format>_<scenario>`. They share the single
//! image handler (decode → paint region → re-encode) and OCR enrichment;
//! shared helpers live here as the family grows.
#![allow(dead_code)]

mod jpeg_exif;
mod png_exif;
mod png_round_trip;
mod tiff_exif;
