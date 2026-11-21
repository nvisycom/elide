//! The PDFium binding and its dedicated render thread.
//!
//! PDFium is not thread-safe and an open document borrows the binding, so all
//! rendering is serialised on a dedicated single-thread pool; the binding is
//! created once on first use via a `thread_local!` and reused.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use bytes::Bytes;
use elide_core::{Error, Result};
use elide_image::RasterImage;
use elide_image::modality::ImageFormat;
use elide_image::primitive::Dimensions;
use pdfium_render::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::pdf::Pdf;

/// Most pages a document may have to be read or rendered: a page is rendered
/// to a full bitmap, so an unbounded count is a memory denial of service.
const MAX_PAGES: PdfPageIndex = 10_000;

/// Most pixels one page may render to. A render holds PDFium's four-byte
/// bitmap, a four-byte copy, and a three-byte RGB copy at once, so this bounds
/// one page near 700 MiB; an A2 page at 300 DPI (35 million pixels) fits.
const MAX_PAGE_PIXELS: f32 = 67_108_864.0;

/// Most pixels one call may render across all its pages: about 2,000 Letter
/// pages at 300 DPI. Each page's output is kept until the call returns, so a
/// document of many huge blank pages would otherwise run without bound.
const MAX_DOCUMENT_PIXELS: f32 = 17_179_869_184.0;

/// Dedicated single-thread pool for PDFium operations.
static PDF_POOL: LazyLock<ThreadPool> = LazyLock::new(|| {
    ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name(|_| "pdfium".into())
        .build()
        .expect("failed to create PDFium thread pool")
});

thread_local! {
    static RENDERER: RefCell<Option<Pdfium>> = const { RefCell::new(None) };
}

/// Run `task` with `pdf` loaded on the PDFium thread.
fn with_document<T: Send>(
    pdf: &Pdf,
    task: impl FnOnce(&PdfDocument<'_>) -> Result<T> + Send,
) -> Result<T> {
    let bytes = pdf.bytes();
    PDF_POOL.install(|| {
        RENDERER.with_borrow_mut(|slot| {
            let pdfium = match slot {
                Some(pdfium) => pdfium,
                None => slot.insert(bind()?),
            };
            let document = pdfium
                .load_pdf_from_byte_slice(&bytes, None)
                .map_err(|e| Error::malformed(format!("failed to load PDF: {e}")))?;
            let count = document.pages().len();
            if count > MAX_PAGES {
                return Err(Error::resource_limit(format!(
                    "document has {count} pages, over the {MAX_PAGES}-page render limit"
                )));
            }
            task(&document)
        })
    })
}

/// Bind the PDFium library: the system one, else one beside the binary.
///
/// # Errors
///
/// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
/// if neither can be loaded: the input is not at fault, the build is.
fn bind() -> Result<Pdfium> {
    let bindings = Pdfium::bind_to_system_library().or_else(|system| {
        Pdfium::bind_to_library("libpdfium").map_err(|local| {
            Error::capability_unavailable(format!(
                "failed to load PDFium library: {system}; fallback: {local}"
            ))
        })
    })?;
    Ok(Pdfium::new(bindings))
}

/// The 1-based pages `numbers` rendered at `scale` to PNG images.
///
/// # Errors
///
/// As [`each_page`] and [`RasterImage::encode`].
pub(super) fn render_pages(
    pdf: &Pdf,
    numbers: &BTreeSet<u32>,
    scale: f32,
) -> Result<BTreeMap<u32, Bytes>> {
    let mut out = BTreeMap::new();
    each_page(pdf, numbers, scale, |number, image| {
        out.insert(number, image.encode()?);
        Ok(())
    })?;
    Ok(out)
}

/// Render each of the 1-based pages `numbers` at `scale` and hand it to
/// `visit` as an image encoding to PNG, on the PDFium thread.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
/// document cannot be loaded, a page cannot be rendered, or a number names no
/// page; [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) for
/// a page, or pages together, too large to render; or as `visit`.
pub(crate) fn each_page(
    pdf: &Pdf,
    numbers: &BTreeSet<u32>,
    scale: f32,
    mut visit: impl FnMut(u32, RasterImage) -> Result<()> + Send,
) -> Result<()> {
    with_document(pdf, |document| {
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);
        let mut rendered = 0usize;
        let mut total = 0.0f32;
        for (number, page) in (1u32..).zip(document.pages().iter()) {
            if !numbers.contains(&number) {
                continue;
            }
            let pixels = page.width().value * scale * page.height().value * scale;
            total += pixels;
            if pixels.is_nan() || pixels > MAX_PAGE_PIXELS {
                return Err(Error::resource_limit(format!(
                    "page {number} would render to {pixels} pixels, over the \
                     {MAX_PAGE_PIXELS}-pixel limit"
                )));
            }
            if total > MAX_DOCUMENT_PIXELS {
                return Err(Error::resource_limit(format!(
                    "the pages would render to more than {MAX_DOCUMENT_PIXELS} pixels"
                )));
            }
            let image = page
                .render_with_config(&config)
                .and_then(|bitmap| bitmap.as_image())
                .map(|image| image.into_rgb8())
                .map_err(|e| Error::malformed(format!("failed to render page {number}: {e}")))?;
            let size = Dimensions::new(image.width(), image.height());
            let image = RasterImage::from_rgb(size, image.into_raw(), ImageFormat::Png)
                .ok_or_else(|| Error::malformed(format!("page {number} rendered short")))?;
            visit(number, image)?;
            rendered += 1;
        }
        if rendered == numbers.len() {
            return Ok(());
        }
        Err(Error::malformed(format!(
            "asked to render {} pages, but the document has only {rendered} of them",
            numbers.len()
        )))
    })
}
