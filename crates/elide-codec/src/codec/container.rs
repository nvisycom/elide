//! [`Container`]: a document that holds addressable sub-parts of other
//! modalities (a DOCX's embedded images, a PDF's image XObjects).
//!
//! The codec layer cannot decode or redact those parts, it knows no
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

use crate::LocalId;

/// One addressable sub-part of a [`Container`].
#[derive(Debug, Clone)]
pub struct Part {
    /// This container's *local*, private id for the part, a zip entry name
    /// (`"word/media/image1.png"`) for DOCX, a PDF object reference, a bundle's
    /// filename, … Unique only **within this one container**; it re-finds the
    /// part in [`Container::replace_part`]. The orchestrator composes these
    /// segments into a full tree path when a container nests another (two
    /// containers can share a local id, `word/media/image1.png` in each of two
    /// bundled DOCX, which the path disambiguates).
    pub id: LocalId,
    /// The part's raw, undecoded bytes, what the orchestrator decodes
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
/// [`replace_part`]. A non-container handler simply isn't one, the
/// downcast yields `None`.
///
/// [`parts`]: Container::parts
/// [`replace_part`]: Container::replace_part
pub trait Container: Send + Sync {
    /// The redactable sub-parts, in no particular order. Each is decoded
    /// and driven independently by the orchestrator.
    ///
    /// **Stable snapshot.** `parts()` must be a side-effect-free view of the
    /// container's *immutable source*, returning the same parts (same id,
    /// bytes, and hint) every call until [`replace_part`] changes one. The
    /// orchestrator relies on this: it may decode a part during analysis and
    /// then decode it *again* at apply time (for a report rebuilt out of
    /// band, with no cached handle), and both decodes must see identical
    /// bytes. A `replace_part` must not alter what a *later* `parts()`
    /// reports for *other* ids, and the redacted bytes a part holds must
    /// surface only through the container's own re-encode, never back through
    /// `parts()`.
    ///
    /// [`replace_part`]: Container::replace_part
    fn parts(&self) -> Vec<Part>;

    /// Replace the part with this container's local `id` with `bytes` (its
    /// redacted form), to be folded in when the container re-encodes. `id` is a
    /// [`Part::id`], this container's own segment, never a full tree path.
    /// Unknown ids are an error so a caller can't silently lose a redaction.
    fn replace_part(&mut self, id: &LocalId, bytes: Bytes) -> Result<()>;
}
