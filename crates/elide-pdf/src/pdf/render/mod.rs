//! Rendering a PDF through PDFium, behind the `render` feature: its pages as
//! pixels. It requires the PDFium shared library at runtime (see
//! `scripts/install-pdfium.sh`).

mod pdfium;

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use elide_core::Result;

pub(crate) use self::pdfium::each_page;
use super::{Pdf, Read};

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
