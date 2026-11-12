//! Decoding raw bytes into a [`Document`].
//!
//! - [`Loader`]: per-modality decoder a leaf format implementation writes.
//!   Returns a concrete [`Stream`].
//! - [`DocumentLoader`]: the whole-document decoder the `FormatRegistry` holds
//!   behind `Arc`; it produces the whole [`Document`]. `LeafLoader` adapts a
//!   [`Loader`] into one for a leaf format.

use elide_core::Result;
use elide_core::modality::Modality;
use elide_core::redaction::HiddenContentPolicy;

use super::{Document, Encode, Stream};
use crate::content::ContentData;

/// A per-modality loader for a leaf format: decodes bytes into one stream.
///
/// A loader validates and parses raw content for its [`Modality`], producing a
/// single [`Stream`]. Leaf formats with no hidden content (a `.txt`, a JSON or
/// CSV file) implement this; [`Format::new`] turns it into a format whose
/// documents have that one stream as their only part. A multi-part format, or
/// one that carries metadata or other hidden content to sanitize under a
/// [`HiddenContentPolicy`], implements [`DocumentLoader`] directly instead.
///
/// # Implementing a third-party leaf format
///
/// 1. Implement [`Stream`](super::Stream) and [`Encode`] for the per-format type
///    that owns the parsed in-memory representation.
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

    /// The stream type this loader produces: the whole document, so it
    /// [encodes](Encode) itself.
    type Stream: Stream<Self::Modality> + Encode;

    /// Validate and parse the content, returning the loaded stream.
    ///
    /// # Errors
    ///
    /// Returns an error when the content is malformed for this format.
    async fn decode(&self, content: ContentData) -> Result<Self::Stream>;
}

/// Decodes raw bytes into a whole [`Document`], sanitizing hidden content.
///
/// Where a [`Loader`] produces a single [`Stream`](super::Stream), a
/// `DocumentLoader` produces the whole [`Document`]: the format's
/// [`Native`](super::Native) document, which owns its streams, and the blobs
/// embedded in it.
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
/// Its one stream becomes a [`LeafDocument`](super::LeafDocument): the bridge
/// for a leaf format that has only a [`Loader`] and no embedded files. Such a
/// format carries no hidden content, so the [`HiddenContentPolicy`] has nothing
/// to act on; a format with hidden content implements [`DocumentLoader`]
/// itself.
pub(crate) struct LeafLoader<L>(pub(crate) L);

#[async_trait::async_trait]
impl<L: Loader> DocumentLoader for LeafLoader<L> {
    async fn decode(&self, content: ContentData, _policy: HiddenContentPolicy) -> Result<Document> {
        let stream = Loader::decode(&self.0, content).await?;
        let format_id = Stream::format(&stream);
        Ok(Document::leaf(format_id, stream))
    }
}
