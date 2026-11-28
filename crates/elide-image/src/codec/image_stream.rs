//! [`PixelStream`]: the pixel body part — reads the frame as one chunk, redacts
//! regions in place, and re-encodes just the pixels.

use elide_codec::{FormatId, Stream};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Component, Result};

use super::image_state::ImageState;
use crate::modality::{Image, ImageData, ImageLocation};

/// The pixel stream: yields the whole frame as one chunk and redacts regions in
/// place on the shared [`ImageState`], which the `#exif` stream edits too and
/// the image's document encodes.
#[derive(Component)]
pub(super) struct PixelStream {
    pub(super) state: ImageState,
    #[component(name)]
    pub(super) format_id: FormatId,
}

impl Stream<Image> for PixelStream {
    fn chunks(&self) -> Result<Vec<Chunk<Image>>> {
        Ok(vec![Chunk {
            location: self.state.frame(),
            data: self.state.data()?,
            hints: Vec::new(),
        }])
    }
}

#[async_trait::async_trait]
impl DataReader<Image> for PixelStream {
    async fn read_at(&self, location: &ImageLocation) -> Result<ImageData> {
        self.state.read_at(location)
    }
}

#[async_trait::async_trait]
impl DataWriter<Image> for PixelStream {
    async fn write_at(&mut self, redactions: Redactions<Image>) -> Result<()> {
        self.state.redact(redactions)
    }
}
