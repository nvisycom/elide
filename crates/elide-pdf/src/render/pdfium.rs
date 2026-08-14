//! The PDFium binding and its dedicated render thread.
//!
//! PDFium is not thread-safe and an open document borrows the binding, so all
//! rendering is serialised on a dedicated single-thread pool; the binding is
//! created once on first use via a `thread_local!` and reused.

use std::cell::RefCell;
use std::sync::LazyLock;

use image::{GenericImageView, ImageFormat};
use pdfium_render::prelude::*;

use super::RenderedPage;
use crate::error::{Error, Result};

/// Dedicated single-thread pool for PDFium operations.
static PDF_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name(|_| "pdfium".into())
        .build()
        .expect("failed to create PDFium thread pool")
});

thread_local! {
    static RENDERER: RefCell<Option<Binding>> = const { RefCell::new(None) };
}

/// Render every page of `pdf_bytes` to a [`RenderedPage`] at `scale`, on the
/// dedicated PDFium thread where the binding is valid.
pub(super) fn render(pdf_bytes: Vec<u8>, scale: f32) -> Result<Vec<RenderedPage>> {
    PDF_POOL.install(move || {
        RENDERER.with_borrow_mut(|slot| {
            if slot.is_none() {
                *slot = Some(Binding::new()?);
            }
            slot.as_ref().unwrap().render_all(&pdf_bytes, scale)
        })
    })
}

/// A PDFium binding, lazily initialised on the dedicated render thread.
struct Binding {
    pdfium: Pdfium,
}

impl Binding {
    fn new() -> Result<Self> {
        let bindings = Pdfium::bind_to_system_library()
            .or_else(|_| Pdfium::bind_to_library("libpdfium"))
            .map_err(|e| Error::invalid_document(format!("failed to load PDFium library: {e}")))?;
        Ok(Self {
            pdfium: Pdfium::new(bindings),
        })
    }

    fn render_all(&self, pdf_bytes: &[u8], scale: f32) -> Result<Vec<RenderedPage>> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| Error::invalid_document(format!("failed to load PDF: {e}")))?;
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);

        let mut pages = Vec::new();
        for page in document.pages().iter() {
            let bitmap = page
                .render_with_config(&config)
                .map_err(|e| Error::invalid_document(format!("failed to render PDF page: {e}")))?;
            let image = bitmap.as_image().map_err(|e| {
                Error::invalid_document(format!("failed to convert PDF page bitmap: {e}"))
            })?;
            let (width, height) = image.dimensions();
            let mut png = std::io::Cursor::new(Vec::new());
            image
                .write_to(&mut png, ImageFormat::Png)
                .map_err(|e| Error::invalid_document(format!("failed to encode page PNG: {e}")))?;
            pages.push(RenderedPage {
                png: png.into_inner(),
                width,
                height,
            });
        }
        Ok(pages)
    }
}
