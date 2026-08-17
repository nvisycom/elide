//! End-to-end suite for the raster image formats (PNG, JPEG, TIFF). They
//! share one image handler and the OCR enrichment path, so they share one
//! test binary. Each format is a scenario module under `format_image/`.

#[path = "support/mod.rs"]
mod support;

#[path = "format_image/mod.rs"]
mod format_image;
