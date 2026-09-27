//! [`ImageRecombine`]: fold the redacted pixels and the `#exif` blob into one
//! image.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{EncodedPart, Recombine};
use elide_core::Result;

use super::image_state::ImageState;
use super::{EXIF_PART_ID, PIXEL_PART_ID};
use crate::exif::ExifPolicy;

/// The recombiner: fold the redacted pixels (from the shared buffer) and the
/// `#exif` blob into one image.
///
/// The `#exif` blob is the image's own bytes at decode. When a metadata pipeline
/// redacted it, its bytes differ from `original_exif`, so the redacted pixels are
/// laid over that metadata-stripped container (`encode_over_metadata`). When the
/// blob is untouched — its bytes still equal `original_exif` — no metadata
/// pipeline ran, so the fallback [`ExifPolicy`] governs the metadata instead.
pub(super) struct ImageRecombine {
    pub(super) state: ImageState,
    pub(super) policy: ExifPolicy,
    /// The `#exif` blob's bytes at decode, to detect whether it was redacted.
    pub(super) original_exif: Bytes,
}

impl Recombine for ImageRecombine {
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData> {
        // A metadata pipeline redacts the `#exif` blob in place, changing its
        // bytes; only then does its container matter here.
        let exif = parts.iter().find(|p| p.id.as_str() == EXIF_PART_ID);
        let bytes = if exif.is_some_and(|p| p.bytes != self.original_exif) {
            // A metadata pipeline stripped the `#exif` container; lay the redacted
            // pixels over it. The pixel part's own encoding used the fallback
            // policy, so it can't be reused: the pixels must be re-laid over this
            // stripped container instead.
            let container = &exif.expect("checked present").bytes;
            self.state.encode_over_metadata(container)?
        } else {
            // Untouched metadata: the fallback policy governs it, which is exactly
            // how `PixelStream::encode` already encoded the pixel body part. Reuse
            // those bytes rather than encoding the image a second time.
            parts
                .iter()
                .find(|p| p.id.as_str() == PIXEL_PART_ID)
                .map(|p| p.bytes.clone())
                .map(Ok)
                .unwrap_or_else(|| self.state.encode(self.policy))?
        };
        Ok(ContentData::new(bytes))
    }
}
