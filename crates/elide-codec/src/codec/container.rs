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

use std::borrow::Cow;
use std::fmt;

use bytes::Bytes;
use elide_core::Result;

/// Opaque identifier for a [`Part`] within its [`Container`].
///
/// Container-private: the value is whatever the container uses to re-find
/// the part — a zip entry name (`"word/media/image1.png"`) for DOCX, a PDF
/// object reference, … The orchestrator never inspects it; it only carries
/// the id between [`Container::parts`] and [`Container::replace_part`] and
/// keys the report's parts by it. Modelled as an opaque newtype (mirroring
/// [`FormatId`]) so that container-private string stays out of the
/// orchestrator's type signatures and a caller can't accidentally pass a
/// bare string where a part id is meant.
///
/// [`FormatId`]: super::FormatId
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartId(Cow<'static, str>);

impl PartId {
    /// Construct from a static string literal, with no allocation.
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// Borrow as `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PartId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for PartId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for PartId {
    fn from(id: String) -> Self {
        Self(Cow::Owned(id))
    }
}

impl From<&'static str> for PartId {
    fn from(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }
}

/// One addressable sub-part of a [`Container`].
#[derive(Debug, Clone)]
pub struct Part {
    /// Container-private identifier the replacement is keyed on (a zip
    /// entry name, a PDF object reference, …). Opaque to the orchestrator.
    pub id: PartId,
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

    /// Replace the part identified by `id` with `bytes` (its redacted
    /// form), to be folded in when the container re-encodes. Unknown ids
    /// are an error so a caller can't silently lose a redaction.
    fn replace_part(&mut self, id: &PartId, bytes: Bytes) -> Result<()>;
}
