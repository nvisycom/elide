//! [`Container`]: a document that holds addressable sub-parts of other
//! modalities (a DOCX's embedded images, a PDF's image XObjects).
//!
//! The codec layer cannot decode or redact those parts — it knows no
//! recognizers and cannot reach the [`FormatRegistry`]. So a container
//! only *exposes* its parts as opaque byte-blobs and *accepts* redacted
//! bytes back; the toolkit's orchestrator decodes each part, drives the
//! right modality pipeline over it, and writes the result back by id.
//!
//! Modality-neutral by construction: a [`Part`] is `(id, bytes, hint)`,
//! so a zip-entry container (DOCX) and a region/object container (PDF)
//! present the same surface even though their internals differ.
//!
//! [`FormatRegistry`]: crate::FormatRegistry

use bytes::Bytes;
use elide_core::Result;

/// One addressable sub-part of a [`Container`].
#[derive(Debug, Clone)]
pub struct Part {
    /// Container-private identifier the replacement is keyed on (a zip
    /// entry name, a PDF object reference, …). Opaque to the orchestrator.
    pub id: String,
    /// The part's raw, undecoded bytes — what the orchestrator decodes
    /// through the registry.
    pub bytes: Bytes,
    /// A hint at the part's modality/format for the orchestrator to
    /// resolve a decoder: a filename extension (`"png"`) or content-type.
    /// Empty when the container can't say.
    pub hint: String,
}

/// A document with addressable sub-parts of (possibly) other modalities.
///
/// Implemented by container handlers (DOCX, ahead PDF). The orchestrator
/// downcasts an erased handle to `&mut dyn Container`, lists [`parts`],
/// redacts each out-of-band, and feeds results back through
/// [`replace_part`]. A non-container handler simply isn't one — the
/// downcast yields `None`.
///
/// [`parts`]: Container::parts
/// [`replace_part`]: Container::replace_part
pub trait Container: Send + Sync {
    /// The redactable sub-parts, in no particular order. Each is decoded
    /// and driven independently by the orchestrator.
    fn parts(&self) -> Vec<Part>;

    /// Replace the part identified by `id` with `bytes` (its redacted
    /// form), to be folded in when the container re-encodes. Unknown ids
    /// are an error so a caller can't silently lose a redaction.
    fn replace_part(&mut self, id: &str, bytes: Bytes) -> Result<()>;
}
