//! [`ImageDocument`]: a decoded image, its pixel and `#exif` streams encoded
//! as one image, then its metadata checked.

use std::iter;

use elide_codec::content::ContentData;
use elide_codec::{Blob, LocalId, Native, StreamMut};
use elide_core::Result;
use elide_core::modality::metadata::Metadata;
use elide_core::redaction::Disposition;

use super::exif_stream::ExifStream;
use super::image_stream::PixelStream;
use super::{EXIF_PART_ID, PIXEL_PART_ID};
use crate::exif::Source;
use crate::modality::{Image, ImageFormat};

/// A decoded image: its pixel stream and, when metadata is treated as content,
/// its `#exif` stream, both over one buffer. [`encode`](Native::encode) encodes
/// that buffer, which carries both the pixel redactions and the field edits, as
/// one image.
///
/// The decode already decided the metadata: under
/// [`KeepOnlyEssential`](Disposition::KeepOnlyEssential) the buffer's source was
/// stripped, and under [`ScanAsContent`](Disposition::ScanAsContent) the
/// `#exif` stream edits the fields detection picked.
pub(super) struct ImageDocument {
    /// The pixels.
    pub(super) pixels: PixelStream,
    /// The surfaced EXIF fields, over the pixel stream's buffer.
    pub(super) exif: Option<ExifStream>,
    /// The image's container format, to read the encoded output's metadata.
    pub(super) format: ImageFormat,
    /// The metadata disposition the decode applied, which the output must
    /// honour.
    pub(super) metadata: Disposition,
}

impl Native for ImageDocument {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        let pixels = (
            LocalId::new(PIXEL_PART_ID),
            StreamMut::new::<Image, _>(&mut self.pixels),
        );
        let exif = self.exif.as_mut().map(|exif| {
            (
                LocalId::new(EXIF_PART_ID),
                StreamMut::new::<Metadata, _>(exif),
            )
        });
        iter::once(pixels).chain(exif).collect()
    }

    fn encode(&self, _blobs: &[Blob], _removed: &[LocalId]) -> Result<ContentData> {
        Ok(ContentData::new(self.pixels.state.encode()?))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        let source = Source::new(output.as_bytes(), self.format.to_exif());
        source.check_stripped(self.metadata.treats_as_content())
    }
}
