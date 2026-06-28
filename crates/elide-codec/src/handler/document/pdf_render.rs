//! PDF-to-image rendering via PDFium.
//!
//! PDFium is not thread-safe and an open `PdfDocument` borrows the binding,
//! so all rendering is serialised on a dedicated single-thread
//! [`ThreadPool`]; the [`PdfRenderer`] binding is created once on first use
//! via a `thread_local!` and reused. Because the document cannot leave that
//! thread, pages are streamed via a callback ([`render_each`]) that runs on
//! the render thread rather than handed back as a lazy iterator.
//!
//! Requires the PDFium shared library to be available at runtime — see
//! `scripts/install-pdfium.sh`. The whole module is behind the
//! `pdf-render` feature so the default build needs no native library.
//!
//! [`ThreadPool`]: rayon::ThreadPool

use std::cell::RefCell;
use std::fmt;
use std::sync::LazyLock;

use elide_core::modality::image::ImageData;
use elide_core::primitive::{Dimensions, Dpi};
use elide_core::{Error, ErrorKind, Result};
use image::{DynamicImage, GenericImageView, ImageFormat};
use pdfium_render::prelude::*;

use crate::handler::image::macros::encode_image;

/// Dedicated single-thread pool for PDFium operations.
static PDF_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name(|_| "pdfium".into())
        .build()
        .expect("failed to create PDFium thread pool")
});

thread_local! {
    static RENDERER: RefCell<Option<PdfRenderer>> = const { RefCell::new(None) };
}

/// Build a validation error from a PDFium failure.
fn pdf_error(context: &str, err: impl fmt::Display) -> Error {
    Error::new(ErrorKind::Validation, format!("{context}: {err}"))
}

/// Render every page of a PDF to a PNG-encoded [`ImageData`] at `dpi`,
/// invoking `on_page` with each page's zero-based index as it is produced.
///
/// The entry point the PDF loader calls under [`OcrMode::Force`]. Rendering
/// streams: the document is opened once and each page is rendered, encoded,
/// and handed to `on_page` before the next is rendered, so peak memory is a
/// single page rather than the whole document. Returning an error from
/// `on_page` (e.g. via `?`) stops rendering early.
///
/// The whole loop runs on the dedicated PDFium thread, where the binding is
/// valid; `on_page` therefore runs there too and must be [`Send`]. Requires
/// the PDFium shared library at runtime (see `scripts/install-pdfium.sh`).
///
/// [`OcrMode::Force`]: super::OcrMode::Force
pub(crate) fn render_each(
    pdf_bytes: &[u8],
    dpi: Dpi,
    on_page: impl FnMut(usize, ImageData) -> Result<()> + Send,
) -> Result<()> {
    let bytes = pdf_bytes.to_vec();

    PDF_POOL.install(move || {
        RENDERER.with_borrow_mut(|slot| {
            if slot.is_none() {
                *slot = Some(PdfRenderer::new()?);
            }
            slot.as_ref().unwrap().render_each(&bytes, dpi, on_page)
        })
    })
}

/// Collect every rendered page into a `Vec`, holding the whole document in
/// memory. A convenience over [`render_each`] for callers that want all
/// pages at once; prefer `render_each` for large documents.
pub(crate) fn render_pages(pdf_bytes: &[u8], dpi: Dpi) -> Result<Vec<ImageData>> {
    let mut pages = Vec::new();
    render_each(pdf_bytes, dpi, |_, page| {
        pages.push(page);
        Ok(())
    })?;
    Ok(pages)
}

/// Encode one rendered page to PNG bytes paired with its pixel dimensions.
fn page_to_image_data(page: &DynamicImage) -> Result<ImageData> {
    let (width, height) = page.dimensions();
    let bytes = encode_image(page, ImageFormat::Png)?;
    Ok(ImageData::new(bytes, Dimensions::new(width, height)))
}

/// Renders PDF pages to images for OCR processing.
///
/// Binding to the PDFium shared library is expensive, so the renderer is
/// lazily initialised on a dedicated thread and reused across calls.
/// Requires the PDFium shared library to be available at runtime (bundled
/// in the deployment image or installed on the host).
pub(crate) struct PdfRenderer {
    pdfium: Pdfium,
}

impl PdfRenderer {
    /// Create a new renderer by binding to a system-provided PDFium library.
    fn new() -> Result<Self> {
        let bindings = Pdfium::bind_to_system_library()
            .or_else(|_| Pdfium::bind_to_library("libpdfium"))
            .map_err(|e| pdf_error("failed to load PDFium library", e))?;
        Ok(Self {
            pdfium: Pdfium::new(bindings),
        })
    }

    /// Render each page in turn using the bound PDFium instance, handing it
    /// to `on_page` before rendering the next.
    fn render_each(
        &self,
        pdf_bytes: &[u8],
        dpi: Dpi,
        mut on_page: impl FnMut(usize, ImageData) -> Result<()>,
    ) -> Result<()> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| pdf_error("failed to load PDF", e))?;

        let config = PdfRenderConfig::new().scale_page_by_factor(dpi.scale_factor());

        for (index, page) in document.pages().iter().enumerate() {
            let bitmap = page
                .render_with_config(&config)
                .map_err(|e| pdf_error("failed to render PDF page", e))?;
            let image = bitmap
                .as_image()
                .map_err(|e| pdf_error("failed to convert PDF page bitmap", e))?;
            on_page(index, page_to_image_data(&image)?)?;
        }

        Ok(())
    }
}
