//! Rendering a PDF through PDFium, behind the `render` feature: its pages as
//! pixels, and its text as PDFium reads it. It requires the PDFium shared
//! library at runtime (see `scripts/install-pdfium.sh`).

mod geometry;
mod pdfium;

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use elide_core::{Error, Result};

pub(crate) use self::geometry::PageObservation;
pub(crate) use self::pdfium::each_page;
use super::{Pdf, Read};
use crate::pdf::reader::{PageFrame, page_frame};

/// The pages `pages` (1-based) rendered at `scale` to PNG images, keyed by
/// page number.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// PDFium cannot load or render the document (or the native library is
/// unavailable), or
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if it has
/// too many pages or they render too large.
pub(crate) struct Renders {
    pub(crate) pages: BTreeSet<u32>,
    pub(crate) scale: f32,
}

impl Read for Renders {
    type Output = BTreeMap<u32, Bytes>;

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        pdfium::render_pages(pdf, &self.pages, self.scale)
    }
}

/// Every page's text as PDFium reads it, with each glyph's box and the
/// page's frame: the input to raster redaction.
///
/// # Errors
///
/// As [`Renders`], and
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) for a
/// glyph whose box cannot be read, or a page count PDFium and the parser
/// disagree on.
pub(crate) struct Observation;

impl Read for Observation {
    type Output = Vec<PageObservation>;

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        let doc = pdf.doc();
        let pages = doc.get_pages();
        let observed = pdfium::observe(pdf)?;
        if observed.len() != pages.len() {
            return Err(Error::malformed(format!(
                "PDFium reads {} pages where the document has {}",
                observed.len(),
                pages.len()
            )));
        }
        Ok(pages
            .into_iter()
            .zip(observed)
            .map(|((page, id), (text, glyphs))| PageObservation {
                page,
                text,
                glyphs,
                frame: page_frame(doc, id).unwrap_or(PageFrame::LETTER),
            })
            .collect())
    }
}
