//! [`OoxmlRecombine`]: re-pack an OOXML document from its redacted body items and
//! blob parts, then check the package it produced.

use std::marker::PhantomData;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::SharedSplice;
use elide_codec::{EncodedPart, LocalId, Recombine};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::{BODY_PART_ID, OoxmlAddress, OoxmlCodec, replacements, without_removed};
use crate::ooxml::{OoxmlFormat, OoxmlPackage};
use crate::opc::check::check;
use crate::opc::{PartPath, PartReplacement};

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
    /// The policy the decode sanitized under, which the output must pass.
    pub(super) policy: HiddenContentPolicy,
    pub(super) _codec: PhantomData<C>,
}

impl<C: OoxmlCodec> Recombine for OoxmlRecombine<C> {
    fn assemble(&self, parts: &[EncodedPart], removed: &[LocalId]) -> Result<ContentData> {
        // Each body block's (current) value overwrites its source byte span in
        // its part's XML.
        let text_replacements = self.state.with_items(replacements);
        // Every embedding's and property part's id is its zip entry path; its
        // (possibly redacted) bytes travel as a part replacement. An unredacted part's bytes equal
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
        let out = without_removed(out, removed)?;
        Ok(ContentData::new(Bytes::from(out)))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        check(output.as_bytes(), self.policy, &C::Format::classifier())
    }
}
