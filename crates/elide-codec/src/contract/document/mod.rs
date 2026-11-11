//! [`Document`]: a decoded document, its format's [`Native`] document (which
//! owns the streams) plus the [`Blob`]s embedded in it.
//!
//! A leaf (a `.txt`) is one stream; a container (a `.docx`) is a body stream,
//! perhaps others (document properties), and blobs (embedded images and files).
//! A stream is redacted in place through the [`StreamMut`] its native document
//! lends; a blob is re-decoded through the registry into its own child
//! `Document`, redacted, and folded back. [`Document::encode`] writes the
//! (possibly redacted) document into native bytes through its [`Native`]
//! document; the engine re-encodes blob children first (post-order), so a nested
//! container is written before the level above needs its bytes.

mod local_id;
mod native;
mod stripped;

use std::fmt;

use bytes::Bytes;
use elide_core::modality::Modality;
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};

pub use self::local_id::LocalId;
pub use self::native::{Encode, LeafDocument, Native};
pub use self::stripped::{Stripped, StrippedContent};
use super::{FormatId, Stream, StreamMut, TypedStream};
use crate::content::ContentData;

/// A decoded document: its format's [`Native`] document and the blobs embedded
/// in it.
pub struct Document {
    format_id: FormatId,
    native: Box<dyn Native>,
    blobs: Vec<Blob>,
    /// Blobs dropped since decode, removed from the output on encode.
    removed: Vec<LocalId>,
    /// What decode stripped from the content.
    stripped: StrippedContent,
    /// The policy the document's hidden content was handled under.
    policy: HiddenContentPolicy,
}

/// A self-contained file embedded in a document (an image in a DOCX, a PDF
/// attachment), re-decoded through the registry into its own [`Document`]. Its
/// redacted bytes fold back through [`Document::replace_blob`].
#[derive(Clone)]
pub struct Blob {
    /// The blob's document-local id.
    pub id: LocalId,
    /// The blob's current bytes: the original, or the redacted bytes
    /// [`Document::replace_blob`] staged.
    pub bytes: Bytes,
    /// A hint at the blob's format (a filename extension); empty when the
    /// container can't say.
    pub hint: String,
}

impl Document {
    /// Build a document from its format id, its `native` document, and the
    /// `blobs` embedded in it.
    pub fn new(format_id: FormatId, native: impl Native + 'static, blobs: Vec<Blob>) -> Self {
        Self {
            format_id,
            native: Box::new(native),
            blobs,
            removed: Vec::new(),
            stripped: StrippedContent::new(),
            policy: HiddenContentPolicy::default(),
        }
    }

    /// Build a leaf document: one `stream` that [encodes](Encode) itself into
    /// the document's bytes (a [`LeafDocument`]). For a single-modality format
    /// with no embedded files (TXT, JSON, CSV).
    pub fn leaf<M: Modality, S: Stream<M> + Encode>(format_id: FormatId, stream: S) -> Self {
        Self::new(format_id, LeafDocument::<M, S>::new(stream), Vec::new())
    }

    /// This document with the record of what its decode stripped.
    #[must_use]
    pub fn with_stripped(mut self, stripped: StrippedContent) -> Self {
        self.stripped = stripped;
        self
    }

    /// What decode stripped from the content.
    pub fn stripped(&self) -> &StrippedContent {
        &self.stripped
    }

    /// Whether decode changed the content: it stripped something, so the
    /// document differs from its input bytes. Such a document is re-encoded
    /// even when detection changes nothing in it, so a nested one folds its
    /// stripped bytes back into its parent instead of the original.
    pub fn changed_at_decode(&self) -> bool {
        !self.stripped.is_empty()
    }

    /// The [`HiddenContentPolicy`] the document was decoded under, which every
    /// document embedded in it is decoded under too, so one policy governs the
    /// whole tree. [`Format::decode`](crate::Format::decode) records it.
    pub fn hidden_content_policy(&self) -> HiddenContentPolicy {
        self.policy
    }

    /// Record `policy` as the one this document was decoded under.
    pub(crate) fn decoded_under(mut self, policy: HiddenContentPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The [`FormatId`] of the producing loader.
    pub fn format_id(&self) -> FormatId {
        self.format_id
    }

    /// The document's streams, each with its document-local id, to read and
    /// redact in place.
    pub fn streams_mut(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        self.native.streams()
    }

    /// The stream `id` as a [`TypedStream<M>`], or `None` when the document has
    /// no stream `id` of modality `M`. A leaf document's one stream is `body`.
    pub fn stream_mut<M: Modality>(&mut self, id: &str) -> Option<TypedStream<'_, M>> {
        let (_, stream) = self
            .native
            .streams()
            .into_iter()
            .find(|(part, _)| *part == id)?;
        stream.downcast()
    }

    /// The blobs embedded in the document.
    pub fn blobs(&self) -> &[Blob] {
        &self.blobs
    }

    /// Stage the redacted `bytes` for the blob `id`, to be folded in on
    /// [`encode`](Self::encode).
    ///
    /// The orchestrator decodes a blob into its own child document, redacts it,
    /// re-encodes it, and hands the result back here.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`](elide_core::ErrorKind::NotFound) for an id that
    /// names no blob, so a caller can't silently lose a redaction.
    pub fn replace_blob(&mut self, id: &LocalId, bytes: Bytes) -> Result<()> {
        let blob = self
            .blobs
            .iter_mut()
            .find(|blob| blob.id == *id)
            .ok_or_else(|| Error::not_found(format!("`{id}` is not a blob of this document")))?;
        blob.bytes = bytes;
        Ok(())
    }

    /// Drop the blob `id`, removing it from the output on
    /// [`encode`](Self::encode).
    ///
    /// The orchestrator drops a blob no codec can decode: content it cannot read
    /// it cannot redact, so the embedding is removed rather than shipped as-is.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`](elide_core::ErrorKind::NotFound) for an id that
    /// names no blob.
    pub fn remove_blob(&mut self, id: &LocalId) -> Result<()> {
        let index = self
            .blobs
            .iter()
            .position(|blob| blob.id == *id)
            .ok_or_else(|| Error::not_found(format!("`{id}` is not a blob of this document")))?;
        self.blobs.remove(index);
        self.removed.push(id.clone());
        Ok(())
    }

    /// Write the (possibly redacted) document into native bytes and validate
    /// the result.
    ///
    /// The [`Native`] document writes its streams' current content with each
    /// blob's current bytes (its staged redaction if
    /// [`replace_blob`](Self::replace_blob) set one, else the original); each
    /// [removed](Self::remove_blob) blob is dropped. It then
    /// [validates](Native::validate) the output.
    ///
    /// A blob's *own* redaction is folded in before this is called (the
    /// orchestrator re-encodes each blob's child document first — post-order),
    /// so `encode` never re-decodes: it only writes.
    ///
    /// # Errors
    ///
    /// Propagates the native document's encode error or its validation error.
    pub fn encode(&self) -> Result<ContentData> {
        let output = self.native.encode(&self.blobs, &self.removed)?;
        self.native.validate(&output)?;
        Ok(output)
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("format_id", &self.format_id)
            .field("blobs", &self.blobs)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob")
            .field("id", &self.id)
            .field("bytes", &self.bytes.len())
            .field("hint", &self.hint)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::modality::tabular::Tabular;
    use elide_core::modality::text::Text;

    use super::*;
    use crate::contract::test_support::{JoinBlobs, StrStream};

    fn blob(id: &'static str, bytes: &'static [u8]) -> Blob {
        Blob {
            id: LocalId::new(id),
            bytes: Bytes::from_static(bytes),
            hint: "bin".to_owned(),
        }
    }

    #[test]
    fn a_leaf_document_encodes_its_one_stream() {
        let doc = Document::leaf(
            FormatId::new("elide.test.str"),
            StrStream("hello".to_owned()),
        );
        assert_eq!(doc.encode().unwrap().as_bytes(), b"hello");
    }

    #[test]
    fn a_stream_is_found_by_id_and_modality() {
        let mut doc = Document::leaf(FormatId::new("elide.test.str"), StrStream("x".to_owned()));
        assert!(doc.stream_mut::<Text>("body").is_some());
        assert!(doc.stream_mut::<Text>("other").is_none());
        assert!(
            doc.stream_mut::<Tabular>("body").is_none(),
            "a text stream is not found as another modality"
        );
    }

    #[test]
    fn a_lent_stream_recovers_only_its_own_modality() {
        let mut doc = Document::leaf(FormatId::new("elide.test.str"), StrStream("x".to_owned()));
        let mut streams = doc.streams_mut();
        let (id, stream) = &mut streams[0];
        assert_eq!(*id, "body");
        assert!(stream.is::<Text>());
        assert!(stream.downcast_mut::<Text>().is_some());
        assert!(!stream.is::<Tabular>());
        assert!(stream.downcast_mut::<Tabular>().is_none());
    }

    #[test]
    fn a_blob_folds_its_replaced_bytes() {
        let mut doc = Document::new(
            FormatId::new("elide.test.blobs"),
            JoinBlobs,
            vec![blob("a", b"one"), blob("b", b"two")],
        );
        doc.replace_blob(&LocalId::new("a"), Bytes::from_static(b"[REDACTED]"))
            .unwrap();
        assert_eq!(doc.encode().unwrap().as_bytes(), b"[REDACTED]two");
        let missing = doc.replace_blob(&LocalId::new("nope"), Bytes::new());
        assert_eq!(missing.unwrap_err().kind(), ErrorKind::NotFound);
    }

    #[test]
    fn a_removed_blob_is_dropped_on_encode() {
        let mut doc = Document::new(
            FormatId::new("elide.test.blobs"),
            JoinBlobs,
            vec![blob("a", b"one"), blob("b", b"two")],
        );
        doc.remove_blob(&LocalId::new("a")).unwrap();
        assert_eq!(doc.blobs().len(), 1);
        assert_eq!(doc.encode().unwrap().as_bytes(), b"two");
        let missing = doc.remove_blob(&LocalId::new("a"));
        assert_eq!(missing.unwrap_err().kind(), ErrorKind::NotFound);
    }
}
