//! [`PixelStream`]: the pixel body part — reads the frame as one chunk, redacts
//! regions in place, and re-encodes just the pixels.

use elide_codec::content::ContentData;
use elide_codec::{FormatId, Stream};
use elide_core::Result;
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::image_state::ImageState;
use crate::exif::ExifPolicy;
use crate::modality::{Image, ImageData, ImageLocation};
use crate::primitive::{BoundingBox, Dimensions, Point};

/// The pixel stream part: yields the whole frame as one chunk, redacts regions in
/// place on the shared [`ImageState`], and re-encodes just the pixels (its
/// metadata is handled by the `#exif` blob and the recombiner).
pub(super) struct PixelStream {
    pub(super) state: ImageState,
    pub(super) format_id: FormatId,
    pub(super) policy: ExifPolicy,
}

impl Stream<Image> for PixelStream {
    fn format(&self) -> FormatId {
        self.format_id.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::new(self.state.encode(self.policy)?))
    }

    fn chunks(&self) -> Result<Vec<Chunk<Image>>> {
        let dims = self.state.dimensions();
        let bbox = BoundingBox::from_origin(
            Point::new(0.0, 0.0),
            Dimensions::new(dims.width as f64, dims.height as f64),
        );
        let data = self.state.image_data(self.policy)?;
        Ok(vec![Chunk {
            location: ImageLocation::new(bbox),
            data,
            hints: Vec::new(),
        }])
    }
}

#[async_trait::async_trait]
impl DataReader<Image> for PixelStream {
    async fn read_at(&self, location: &ImageLocation) -> Result<Option<ImageData>> {
        self.state.crop_encode(location)
    }
}

#[async_trait::async_trait]
impl DataWriter<Image> for PixelStream {
    async fn write_at(&mut self, redactions: Redactions<Image>) -> Result<()> {
        self.state.redact(redactions);
        Ok(())
    }
}
