//! [`ImageRecombine`]: encode the redacted pixels and the edited metadata as one
//! image, then check its metadata.

use elide_codec::content::ContentData;
use elide_codec::{EncodedPart, LocalId, Recombine};
use elide_core::Result;
use elide_core::redaction::Disposition;

use super::PIXEL_PART_ID;
use super::image_state::ImageState;
use crate::exif::Source;
use crate::modality::ImageFormat;

/// The recombiner: encode the shared buffer, which carries both the pixel
/// stream's redactions and the `#exif` stream's field edits, as one image.
///
/// The decode already decided the metadata: under
/// [`KeepOnlyEssential`](Disposition::KeepOnlyEssential) the buffer's source was
/// stripped, and under [`ScanAsContent`](Disposition::ScanAsContent) the
/// `#exif` stream edits the fields detection picked.
pub(super) struct ImageRecombine {
    pub(super) state: ImageState,
    /// The image's container format, to read the encoded output's metadata.
    pub(super) format: ImageFormat,
    /// The metadata disposition the decode applied, which the output must
    /// honour.
    pub(super) metadata: Disposition,
}

impl Recombine for ImageRecombine {
    fn assemble(&self, parts: &[EncodedPart], _removed: &[LocalId]) -> Result<ContentData> {
        // The pixel part encoded the shared buffer, metadata edits included.
        let bytes = match parts.iter().find(|p| p.id.as_str() == PIXEL_PART_ID) {
            Some(pixels) => pixels.bytes.clone(),
            None => self.state.encode()?,
        };
        Ok(ContentData::new(bytes))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        let source = Source::new(output.as_bytes(), self.format.to_exif());
        source.check_stripped(self.metadata.treats_as_content())
    }
}
