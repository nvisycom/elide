//! [`Document`]: a decoded document as an ordered set of typed [`DocumentPart`]s.
//!
//! A document is its parts. A leaf (a `.txt`) is a document with one
//! [`DocumentPart::Stream`]; a container (a `.docx`) is a stream plus
//! [`DocumentPart::Blob`] sub-files (images, document properties). A `Stream`
//! part is redacted in place through its [`Stream<M>`] handle; a `Blob` part is
//! re-decoded through the registry into its own child `Document`, redacted, and
//! folded back. [`Document::encode`] assembles the (possibly redacted) parts
//! into native bytes through the format's [`Recombine`], recursing into blob
//! children first (post-order), which is how a nested container re-encodes
//! before the level above needs its bytes.

mod local_id;
mod recombine;

use std::fmt;

use bytes::Bytes;
use elide_core::Result;
use elide_core::modality::Modality;

pub use self::local_id::LocalId;
pub use self::recombine::{LeafRecombine, Recombine};
use super::{ErasedStream, FormatId, Stream};
use crate::content::ContentData;

/// A decoded document: an ordered set of typed parts plus the format's
/// recombiner.
pub struct Document {
    format_id: FormatId,
    parts: Vec<DocumentPart>,
    recombine: Box<dyn Recombine>,
}

/// One part of a [`Document`]: a redactable stream, or a re-decodable sub-file.
pub enum DocumentPart {
    /// A redactable modality stream (a text body, image pixels, an audio clip),
    /// redacted in place through its [`Stream<M>`] handle. Modality-erased so a
    /// document's parts of different modalities live in one `Vec`.
    Stream {
        /// This part's document-local id.
        id: LocalId,
        /// The erased stream handle; recover the typed form with
        /// [`ErasedStream::downcast_mut`].
        handle: ErasedStream,
    },
    /// A self-contained sub-file, re-decoded through the registry into its own
    /// [`Document`]. Its redacted bytes fold back through
    /// [`Document::replace_part`].
    Blob {
        /// This part's document-local id.
        id: LocalId,
        /// The sub-file's raw, undecoded bytes.
        bytes: Bytes,
        /// A hint at the sub-file's modality/format (a filename extension or
        /// content type); empty when the format can't say.
        hint: String,
    },
}

/// One assembled part, handed to [`Recombine::assemble`]: its id and its
/// (possibly redacted) encoded bytes.
pub struct EncodedPart {
    /// The part's document-local id.
    pub id: LocalId,
    /// The part's encoded bytes — a redacted stream re-encoded, or a blob's
    /// folded bytes.
    pub bytes: Bytes,
}

impl Document {
    /// Build a leaf document: one [`Stream`] part named `body`, whose bytes are
    /// the document's bytes (a [`LeafRecombine`]). For a single-modality format
    /// with no sub-parts (TXT, JSON, a bare image or audio clip).
    pub fn leaf<M: Modality>(format_id: FormatId, stream: Box<dyn Stream<M>>) -> Self {
        let handle = ErasedStream::new(format_id.clone(), stream);
        Self::new(
            format_id,
            vec![DocumentPart::Stream {
                id: LocalId::new("body"),
                handle,
            }],
            Box::new(LeafRecombine),
        )
    }

    /// Build a document from its format id, parts, and recombiner.
    pub fn new(
        format_id: FormatId,
        parts: Vec<DocumentPart>,
        recombine: Box<dyn Recombine>,
    ) -> Self {
        Self {
            format_id,
            parts,
            recombine,
        }
    }

    /// The [`FormatId`] of the producing loader.
    pub fn format_id(&self) -> &FormatId {
        &self.format_id
    }

    /// The document's parts.
    pub fn parts(&self) -> &[DocumentPart] {
        &self.parts
    }

    /// The document's parts, mutably (to redact a stream part in place).
    pub fn parts_mut(&mut self) -> &mut [DocumentPart] {
        &mut self.parts
    }

    /// Stage the redacted `bytes` for the blob part with document-local `id`, to
    /// be folded in on [`encode`](Self::encode).
    ///
    /// The orchestrator decodes a blob part into its own child document,
    /// redacts it, re-encodes it, and hands the result back here. A stream part
    /// is redacted in place through its handle, not through this.
    ///
    /// # Errors
    ///
    /// An unknown id, or an id naming a stream part, is an error so a caller
    /// can't silently lose a redaction.
    pub fn replace_part(&mut self, id: &LocalId, bytes: Bytes) -> Result<()> {
        for part in &mut self.parts {
            if let DocumentPart::Blob {
                id: blob_id,
                bytes: blob_bytes,
                ..
            } = part
                && blob_id == id
            {
                *blob_bytes = bytes;
                return Ok(());
            }
        }
        Err(elide_core::Error::new(
            elide_core::ErrorKind::MalformedInput,
            format!("replace_part: `{id}` is not a blob part of this document"),
        ))
    }

    /// Assemble the document's (possibly redacted) parts into native bytes.
    ///
    /// Each stream part re-encodes its own current content; each blob part
    /// contributes its current bytes (its staged redaction if
    /// [`replace_part`](Self::replace_part) set one, else the original). The
    /// format's [`Recombine`] joins them into the native container.
    ///
    /// A blob part's *own* redaction is folded in before this is called (the
    /// orchestrator re-encodes each blob's child document first — post-order),
    /// so `encode` never re-decodes: it only assembles.
    ///
    /// # Errors
    ///
    /// Propagates a stream's re-encode error or the recombiner's assembly error.
    pub fn encode(&self) -> Result<ContentData> {
        let mut assembled = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            let (id, bytes) = match part {
                DocumentPart::Stream { id, handle } => (id.clone(), handle.encode()?.into_bytes()),
                DocumentPart::Blob { id, bytes, .. } => (id.clone(), bytes.clone()),
            };
            assembled.push(EncodedPart { id, bytes });
        }
        self.recombine.assemble(&assembled)
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("format_id", &self.format_id)
            .field("parts", &self.parts.len())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for DocumentPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentPart::Stream { id, handle } => f
                .debug_struct("Stream")
                .field("id", id)
                .field("handle", handle)
                .finish(),
            DocumentPart::Blob { id, bytes, hint } => f
                .debug_struct("Blob")
                .field("id", id)
                .field("bytes", &bytes.len())
                .field("hint", hint)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::Text;

    use super::*;
    use crate::contract::test_support::{JoinRecombine, StrStream};

    #[test]
    fn a_leaf_document_encodes_its_one_stream() {
        let stream = ErasedStream::new(
            FormatId::new("elide.test.str"),
            Box::new(StrStream("hello".to_owned())),
        );
        let doc = Document::new(
            FormatId::new("elide.test.str"),
            vec![DocumentPart::Stream {
                id: LocalId::new("body"),
                handle: stream,
            }],
            Box::new(LeafRecombine),
        );
        assert_eq!(doc.encode().unwrap().as_bytes(), b"hello");
    }

    #[test]
    fn erased_stream_recovers_its_modality() {
        let mut stream = ErasedStream::new(
            FormatId::new("elide.test.str"),
            Box::new(StrStream("x".to_owned())),
        );
        assert!(stream.is::<Text>());
        assert!(stream.downcast_mut::<Text>().is_some());
    }

    #[test]
    fn a_blob_part_folds_its_replaced_bytes() {
        let mut doc = Document::new(
            FormatId::new("elide.test.blobs"),
            vec![
                DocumentPart::Blob {
                    id: LocalId::new("a"),
                    bytes: Bytes::from_static(b"one"),
                    hint: "bin".to_owned(),
                },
                DocumentPart::Blob {
                    id: LocalId::new("b"),
                    bytes: Bytes::from_static(b"two"),
                    hint: "bin".to_owned(),
                },
            ],
            Box::new(JoinRecombine),
        );
        doc.replace_part(&LocalId::new("a"), Bytes::from_static(b"[REDACTED]"))
            .unwrap();
        assert_eq!(doc.encode().unwrap().as_bytes(), b"[REDACTED]two");
        assert!(
            doc.replace_part(&LocalId::new("nope"), Bytes::new())
                .is_err()
        );
    }
}
