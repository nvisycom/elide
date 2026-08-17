//! End-to-end PDF suite: the born-digital text-splice path and the raster
//! (image-only) path. The raster scenario needs native PDFium at runtime and
//! is `#[ignore]`d by default; the binary compiles without it.

#[path = "support/mod.rs"]
mod support;

#[path = "format_pdf/mod.rs"]
mod format_pdf;
