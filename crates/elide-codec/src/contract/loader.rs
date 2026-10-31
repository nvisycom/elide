//! Decoding raw bytes into a [`Document`].
//!
//! - [`Loader`]: per-modality decoder a leaf format implementation writes.
//!   Returns a concrete [`Stream`](super::Stream).
//! - [`DocumentLoader`]: the parts-model decoder the `FormatRegistry` holds
//!   behind `Arc`; it produces the whole [`Document`]. [`LeafLoader`] adapts a
//!   [`Loader`] into one for a leaf format.

use elide_core::Result;
use elide_core::modality::Modality;
use elide_core::redaction::HiddenContentPolicy;

use super::{Document, Stream};
use crate::content::ContentData;

/// Per-modality leaf-format loader.
///
/// A loader validates and parses raw content for its [`Modality`], producing a
/// single [`Stream`](super::Stream). Leaf formats with no hidden content (a
/// `.txt`, a JSON or CSV file) implement this; [`Format::new`] wraps it in a
/// [`LeafLoader`] to a one-[`Stream`](super::Stream) [`Document`]. A multi-part
/// format, or one that carries metadata or other hidden content to sanitize
/// under a [`HiddenContentPolicy`], implements [`DocumentLoader`] directly instead.
///
/// # Implementing a third-party leaf format
///
/// 1. Implement [`Stream`](super::Stream) for the per-format type that owns the
///    parsed in-memory representation.
/// 2. Implement `Loader` for a stateless type whose [`decode`] validates raw
///    [`ContentData`] and returns the stream.
/// 3. Build a [`Format`] with [`Format::new`], chain extensions / content types
///    as needed, and register it on a `FormatRegistry`.
///
/// [`Modality`]: Self::Modality
/// [`decode`]: Loader::decode
/// [`Format`]: super::Format
/// [`Format::new`]: super::Format::new
#[async_trait::async_trait]
pub trait Loader: Send + Sync + 'static {
    /// The modality this loader decodes into.
    type Modality: Modality;

    /// The stream type this loader produces.
    type Stream: Stream<Self::Modality>;

    /// Validate and parse the content, returning the loaded stream.
    ///
    /// # Errors
    ///
    /// Returns an error when the content is malformed for this format.
    async fn decode(&self, content: ContentData) -> Result<Self::Stream>;
}

/// Decode raw content into a [`Document`] — the parts model's loader.
///
/// Where a [`Loader`] produces a single [`Stream`](super::Stream), a
/// `DocumentLoader` produces
/// the whole [`Document`]: its stream part(s), any blob sub-parts, and the
/// format's recombiner. A leaf format returns a one-[`Stream`](super::Stream)
/// document with a trivial recombiner.
///
/// Decoding is where a format applies the [`HiddenContentPolicy`]: hidden content
/// the policy strips is gone before any part is built, so detection only sees
/// what the encoded document will carry. A format records what it stripped on
/// the document ([`with_stripped`](Document::with_stripped)), naming each item
/// by the [`HiddenContent`](elide_core::redaction::HiddenContent) vocabulary,
/// so the strip is reported and the document is re-encoded even when detection
/// finds nothing.
#[async_trait::async_trait]
pub trait DocumentLoader: Send + Sync + 'static {
    /// Validate, sanitize under `policy`, and parse the content into a
    /// [`Document`].
    ///
    /// # Errors
    ///
    /// Returns an error when the content is malformed for this format, or holds
    /// structure the format does not recognize.
    async fn decode(&self, content: ContentData, policy: HiddenContentPolicy) -> Result<Document>;
}

/// A [`Loader`] wrapped so it produces a leaf [`Document`].
///
/// Its one handler becomes a stream, with a trivial recombiner: the bridge for a
/// leaf format that has only a [`Loader`] and no sub-parts. Such a format
/// carries no hidden content, so the [`HiddenContentPolicy`] has nothing to act on; a
/// format with hidden content implements [`DocumentLoader`] itself.
pub struct LeafLoader<L>(pub L);

#[async_trait::async_trait]
impl<L: Loader> DocumentLoader for LeafLoader<L> {
    async fn decode(&self, content: ContentData, _policy: HiddenContentPolicy) -> Result<Document> {
        let stream = Loader::decode(&self.0, content).await?;
        let format_id = Stream::format(&stream);
        let stream: Box<dyn Stream<L::Modality>> = Box::new(stream);
        Ok(Document::leaf(format_id, stream))
    }
}
