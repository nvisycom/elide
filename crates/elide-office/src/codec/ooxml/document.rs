//! [`OoxmlDocument`]: a decoded OOXML document (DOCX, PPTX), re-packed from its
//! redacted streams and blobs, then checked.

use std::iter;
use std::marker::PhantomData;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::ExtractStream;
use elide_codec::{Blob, LocalId, Native, StreamMut};
use elide_core::Result;
use elide_core::modality::text::Text;
use elide_core::redaction::HiddenContentPolicy;

use super::{
    BODY_PART_ID, OoxmlAddress, OoxmlCodec, blob_replacements, replacements, without_removed,
};
use crate::codec::props::DocProps;
use crate::ooxml::{OoxmlFormat, OoxmlPackage};
use crate::opc::check::check;

/// A decoded OOXML document: its body text stream and document-property
/// streams over the retained package.
///
/// [`encode`](Native::encode) turns the body items into text replacements, and
/// the property streams and blobs (embeddings) into part replacements, then
/// re-packs through [`rewrite_with_parts`](OoxmlPackage::rewrite_with_parts).
pub(crate) struct OoxmlDocument<C: OoxmlCodec> {
    /// The body's text blocks.
    pub(crate) body: ExtractStream<OoxmlAddress>,
    /// The document-property parts, when metadata is treated as content.
    pub(super) properties: DocProps,
    /// The (sanitized) package bytes, retained so every unredacted part
    /// re-packs unchanged.
    pub(super) archive: Bytes,
    /// The policy the decode sanitized under, which the output must pass.
    pub(super) policy: HiddenContentPolicy,
    pub(super) _codec: PhantomData<C>,
}

impl<C: OoxmlCodec> Native for OoxmlDocument<C> {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        let body = (
            LocalId::new(BODY_PART_ID),
            StreamMut::new::<Text, _>(&mut self.body),
        );
        iter::once(body).chain(self.properties.streams()).collect()
    }

    fn encode(&self, blobs: &[Blob], removed: &[LocalId]) -> Result<ContentData> {
        // Each body block's (current) value overwrites its source byte span in
        // its part's XML. Each property part and embedding is a zip entry named
        // by its id; its (possibly redacted) bytes replace the entry. An
        // unredacted one's bytes equal the original, so its replacement is a
        // no-op re-pack.
        let text = replacements(self.body.items());
        let mut parts = self.properties.replacements();
        parts.extend(blob_replacements(blobs));

        let out = OoxmlPackage::<C::Format>::open(&self.archive)
            .and_then(|pkg| pkg.rewrite_with_parts(&text, &parts))?;
        let out = without_removed(out, removed)?;
        Ok(ContentData::new(Bytes::from(out)))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        check(output.as_bytes(), self.policy, &C::Format::classifier())
    }
}
