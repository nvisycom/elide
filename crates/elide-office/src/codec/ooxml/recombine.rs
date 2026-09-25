//! [`OoxmlRecombine`]: re-pack an OOXML document from its redacted body items and
//! blob parts.

use std::marker::PhantomData;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::SharedSplice;
use elide_codec::{EncodedPart, Recombine};
use elide_core::Result;

use super::{BODY_PART_ID, OoxmlAddress, OoxmlCodec};
use crate::ooxml::OoxmlPackage;
use crate::opc::{PartPath, PartReplacement, Replacement};

/// Re-packs an OOXML document from its redacted parts. Holds the original
/// package bytes and the shared body-item state (redacted in place by the body
/// [`ExtractStream`](elide_codec::extract::ExtractStream));
/// [`assemble`](Recombine::assemble) turns the items into text replacements and
/// the blob [`EncodedPart`]s (embeddings, document properties) into part
/// replacements, then delegates to
/// [`rewrite_with_parts`](OoxmlPackage::rewrite_with_parts).
pub(crate) struct OoxmlRecombine<C: OoxmlCodec> {
    /// The original package bytes, retained so every unredacted part re-packs
    /// unchanged.
    pub(super) archive: Bytes,
    /// The (redacted-in-place) body text blocks, shared with the body stream.
    pub(super) state: SharedSplice<OoxmlAddress>,
    pub(super) _codec: PhantomData<C>,
}

impl<C: OoxmlCodec> Recombine for OoxmlRecombine<C> {
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData> {
        // Each body block's (current) value overwrites its source byte span in
        // its part's XML.
        let text_replacements: Vec<Replacement> = self.state.with_items(|items| {
            items
                .iter()
                .map(|item| Replacement {
                    part: item.address.part.clone(),
                    start: item.address.span.start,
                    end: item.address.span.end,
                    text: item.value.clone().into(),
                })
                .collect()
        });
        // Every blob sub-part's id is its zip entry path; its (possibly redacted)
        // bytes travel as a part replacement. An unredacted part's bytes equal
        // the original, so its replacement is a no-op re-pack. The body stream's
        // own part carries no standalone bytes (the text lands via the
        // replacements above), so it is not a zip part and is skipped.
        let media: Vec<PartReplacement> = parts
            .iter()
            .filter(|part| part.id.as_str() != BODY_PART_ID)
            .map(|part| PartReplacement::new(PartPath::from(part.id.as_str()), part.bytes.to_vec()))
            .collect();

        let out = OoxmlPackage::<C::Format>::open(&self.archive)
            .and_then(|pkg| pkg.rewrite_with_parts(&text_replacements, &media))?;
        Ok(ContentData::new(Bytes::from(out)))
    }
}
