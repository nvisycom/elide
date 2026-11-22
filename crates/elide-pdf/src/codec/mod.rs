//! The codec adapter for PDF: the format, decoded for redaction.
//!
//! A decoded document is a page-text body `Stream<Text>` (`PdfStream`), (with
//! the `render` feature) a `Stream<Image>` per page redacted from its render
//! (`PageStream`), and a [`Blob`](elide_codec::Blob) per embedded image and
//! attachment. A page is redacted one of two ways:
//!
//! - **Glyph deletion** (the default): the text detected on the page's text
//!   layer is deleted from the content streams, keeping a selectable text
//!   layer with the detected spans gone. A removed span's text closes up; a
//!   covered one keeps its place under a black box. A redaction on the body stream is
//!   recorded on its `PdfState` and applied on encode.
//! - **From its render** (feature `render`): the page is rendered, the image
//!   pipeline detects on what it shows (OCR) and redacts the render, and
//!   encode replaces the page by it. This is every page under
//!   [`RasterMode::Always`], and under the default mode a scanned page, or one
//!   that draws text whose glyphs can't be placed (in a font whose widths
//!   can't be known, or written vertically). Its text layer is never read.
//!
//! [`pdf_format`] decodes for glyph deletion; with the `render` feature
//! [`pdf_format_with`] takes an explicit [`RasterMode`].
//!
//! [`RasterMode`]: crate::primitive::RasterMode
//! [`RasterMode::Always`]: crate::primitive::RasterMode::Always

mod blob_id;
mod document;
mod loader;
#[cfg(feature = "render")]
mod page;
mod plan;
mod state;
mod stream;

use elide_codec::{Format, FormatId};

use self::loader::PdfDocumentLoader;
#[cfg(feature = "render")]
use crate::primitive::{Dpi, RasterMode};

/// The stable [`FormatId`] of the PDF format, `elide.document.pdf`.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pdf");

/// The body stream's part id; excluded from the blob-part fold on assemble.
const BODY_PART_ID: &str = "body";

/// The resolution a scanned page is rendered at for OCR, outside
/// [`RasterMode::Always`].
#[cfg(feature = "render")]
const SCAN_DPI: Dpi = Dpi::OCR;

/// The PDF [`Format`], decoding pages for glyph deletion.
///
/// Its metadata (`/Info`, XMP) and collaboration content (annotations,
/// comments) are always stripped and recorded, whatever the
/// [`HiddenContentPolicy`](elide_core::redaction::HiddenContentPolicy): the
/// rebuild models neither as content.
///
/// With the `render` feature, scanned pages are also rendered for OCR, the
/// raster mode [`Auto`](crate::primitive::RasterMode::Auto); build the format
/// with `pdf_format_with` for another mode.
pub fn pdf_format() -> Format {
    format(PdfDocumentLoader::default())
}

/// The PDF [`Format`] under an explicit [`RasterMode`].
///
/// [`Always`](RasterMode::Always) replaces every page by a redacted image of
/// it; [`Auto`](RasterMode::Auto) deletes glyphs and renders only scanned
/// pages; [`Never`](RasterMode::Never) deletes glyphs and renders nothing.
#[cfg(feature = "render")]
#[cfg_attr(docsrs, doc(cfg(feature = "render")))]
pub fn pdf_format_with(raster: RasterMode) -> Format {
    format(PdfDocumentLoader::new(raster))
}

/// The PDF [`Format`] decoding through `loader`.
fn format(loader: PdfDocumentLoader) -> Format {
    Format::with_document_loader(FORMAT_ID, loader)
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}
