//! [`ImageRecombine`]: fold the redacted pixels and the `#exif` blob into one
//! image, then check its metadata.

use elide_codec::content::ContentData;
use elide_codec::{EncodedPart, LocalId, Recombine};
use elide_core::Result;
use elide_core::redaction::Disposition;

use super::image_state::ImageState;
use super::{EXIF_PART_ID, PIXEL_PART_ID};
use crate::exif::Source;
use crate::modality::ImageFormat;

/// The recombiner: fold the redacted pixels (from the shared buffer) and, when
/// metadata is redacted, the `#exif` blob into one image.
///
/// The decode already decided the metadata: under
/// [`Strip`](Disposition::Strip) the buffer's source was stripped, and under
/// [`Redact`](Disposition::Redact) the `#exif` blob carries the container
/// whose fields detection edits. So the pixel part's own encoding is the answer
/// unless an `#exif` blob is present (lay the pixels over its container) or was
/// removed because no codec could read it (strip the metadata instead).
pub(super) struct ImageRecombine {
    pub(super) state: ImageState,
    /// The image's container format, to read the encoded output's metadata.
    pub(super) format: ImageFormat,
    /// The metadata disposition the decode applied, which the output must
    /// honour.
    pub(super) metadata: Disposition,
}

impl Recombine for ImageRecombine {
    fn assemble(&self, parts: &[EncodedPart], removed: &[LocalId]) -> Result<ContentData> {
        let bytes = if let Some(exif) = parts.iter().find(|p| p.id.as_str() == EXIF_PART_ID) {
            self.state.encode_over_metadata(&exif.bytes)?
        } else if removed.iter().any(|id| id.as_str() == EXIF_PART_ID) {
            self.state.encode_without_metadata()?
        } else {
            match parts.iter().find(|p| p.id.as_str() == PIXEL_PART_ID) {
                Some(pixels) => pixels.bytes.clone(),
                None => self.state.encode()?,
            }
        };
        Ok(ContentData::new(bytes))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        let source = Source::new(output.as_bytes(), self.format.to_exif());
        source.check_stripped(self.metadata == Disposition::Redact)
    }
}
