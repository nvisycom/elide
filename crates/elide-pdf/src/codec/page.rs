//! [`PageStream`]: a scanned page's render, an image stream of the document.

use bytes::Bytes;
use elide_codec::{FormatId, LocalId, Stream};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};
use elide_image::ImageBuffer;
use elide_image::modality::{Image, ImageData, ImageFormat, ImageLocation, ImageReplacement};
use elide_image::primitive::BoundingBox;

use super::FORMAT_ID;
use crate::content::page::ScannedPage;

/// A scanned page's render: the image pipeline reads it for OCR and redacts
/// it in place, and each region it redacts is kept so encode can paint it
/// over the page as the document then draws it.
///
/// The render is held as its PNG and decoded only to be read or redacted, so
/// a long scan does not hold every page's pixels at once.
pub(super) struct PageStream {
    /// 1-based page number.
    number: u32,
    /// The page's PNG render, every region redacted on it painted in.
    render: Bytes,
    /// Each region of the render redacted, with how.
    redactions: Vec<(BoundingBox<u32>, ImageReplacement)>,
}

impl PageStream {
    /// Page `number`'s stream over its PNG `render`.
    pub(super) fn new(number: u32, render: Bytes) -> Self {
        Self {
            number,
            render,
            redactions: Vec::new(),
        }
    }

    /// The stream's document-local id, `page-N`.
    pub(super) fn id(&self) -> LocalId {
        LocalId::new(format!("page-{}", self.number))
    }

    /// The page as redacted, or `None` when nothing was.
    pub(super) fn scanned(&self) -> Option<ScannedPage> {
        (!self.redactions.is_empty()).then(|| ScannedPage {
            number: self.number,
            painted: self.render.clone(),
            redactions: self.redactions.clone(),
        })
    }

    /// The render, decoded.
    fn open(&self) -> Result<ImageBuffer> {
        ImageBuffer::open(&self.render, ImageFormat::Png)
    }

    /// Check that `location` addresses this page: it names no page, or
    /// this one.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput)
    /// if it names another page.
    fn on_this_page(&self, location: &ImageLocation) -> Result<()> {
        match location.page {
            Some(page) if page != self.number => Err(Error::malformed(format!(
                "a region of page {page} addressed to page {}'s render",
                self.number
            ))),
            _ => Ok(()),
        }
    }
}

impl Stream<Image> for PageStream {
    fn format(&self) -> FormatId {
        FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Image>>> {
        let render = self.open()?;
        Ok(vec![Chunk {
            location: render.frame().with_page(self.number),
            data: render.data()?,
            hints: Vec::new(),
        }])
    }
}

#[async_trait::async_trait]
impl DataReader<Image> for PageStream {
    async fn read_at(&self, location: &ImageLocation) -> Result<ImageData> {
        self.on_this_page(location)?;
        self.open()?.read_at(location)
    }
}

#[async_trait::async_trait]
impl DataWriter<Image> for PageStream {
    /// Paint each redaction into the render and keep its region; a region
    /// outside the render, or of another page, refuses the whole batch,
    /// painting none of it.
    async fn write_at(&mut self, redactions: Redactions<Image>) -> Result<()> {
        for (location, _) in &redactions {
            self.on_this_page(location)?;
        }
        let mut render = self.open()?;
        let regions = render.redact_all(redactions)?;
        self.render = render.encode()?;
        self.redactions.extend(regions);
        Ok(())
    }
}
