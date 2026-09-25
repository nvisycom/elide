//! [`Stream<M>`]: a redactable modality stream — the content of one
//! [`DocumentPart`](super::DocumentPart).
//!
//! A `Stream` is the read/redact/re-encode surface for one modality's content:
//! a text body, an image's pixels, an audio clip. It is a [`DataReader`] +
//! [`DataWriter`] (random-access read / batch redact, shared with the rest of
//! the workspace) plus the codec-specific stream surface — identify, serialise,
//! chunk, and lift. A document's cross-modality sub-parts are NOT here; they are
//! the [`Document`](super::Document)'s other
//! [`DocumentPart`](super::DocumentPart)s.
//!
//! [`erased`] holds [`ErasedStream`] / [`TypedStream`], the modality-erasure that
//! lets a `Document`'s stream parts of different modalities live in one `Vec`.
//!
//! [`DataReader`]: elide_core::modality::DataReader
//! [`DataWriter`]: elide_core::modality::DataWriter

mod erased;

use elide_core::Result;
use elide_core::modality::{Chunk, DataReader, DataWriter, Modality};

pub use self::erased::{ErasedStream, TypedStream};
use super::FormatId;
use crate::content::ContentData;

/// A redactable modality stream: the content of one [`DocumentPart::Stream`].
///
/// A `Stream` *is* a [`DataReader`] + [`DataWriter`]: random-access read
/// (`read_at`) and batch redaction (`write_at`) come from those shared traits,
/// so a codec-backed stream plugs straight into anything that bounds on them
/// (the toolkit's anonymizer). On top of that base it adds the codec surface:
/// identify and serialise ([`format`], [`encode`]), stream chunks
/// ([`read_next`]), and lift a chunk-local finding back to source coordinates
/// ([`lift`]).
///
/// The stream owns its cursor; concurrent iteration of the same stream is not
/// supported (only one `&mut self`). The one async method (`read_next`) is
/// boxed via `#[async_trait]`, so a stream stores behind `Box<dyn Stream<M>>`.
///
/// [`DataReader`]: elide_core::modality::DataReader
/// [`DataWriter`]: elide_core::modality::DataWriter
/// [`DocumentPart::Stream`]: super::DocumentPart::Stream
/// [`format`]: Stream::format
/// [`encode`]: Stream::encode
/// [`read_next`]: Stream::read_next
/// [`lift`]: Stream::lift
#[async_trait::async_trait]
pub trait Stream<M: Modality>: DataReader<M> + DataWriter<M> + Send + Sync + 'static {
    /// Stable id of the format this stream represents (e.g. `"elide.text.txt"`).
    /// Cheap to clone.
    fn format(&self) -> FormatId;

    /// Serialize the stream's current content back to [`ContentData`] — this
    /// stream's own bytes, not the whole document (the [`Document`]'s
    /// [`Recombine`] assembles the parts).
    ///
    /// # Errors
    ///
    /// Returns an error when the in-memory representation cannot be re-encoded.
    ///
    /// [`Document`]: super::Document
    /// [`Recombine`]: super::Recombine
    fn encode(&self) -> Result<ContentData>;

    /// Advance the cursor and yield the next chunk, or `None` at
    /// end-of-stream.
    async fn read_next(&mut self) -> Result<Option<Chunk<M>>>;

    /// Promote a `local` location, expressed in `chunk`'s own coordinate
    /// system, to a source-global [`M::Location`].
    ///
    /// A recognizer sees a chunk's decoded payload and emits a finding in
    /// *chunk-local* coordinates: a byte range into the chunk text, a box
    /// within the chunk frame, a span within the chunk's clip. Downstream
    /// stages need locations that address the whole source, so the stream
    /// rebases the local one onto the chunk's origin.
    ///
    /// For a chunk that is a byte-for-byte slice of source the mapping is the
    /// identity offset add against `chunk.location`; a stream whose chunks
    /// decode escapes (JSON) walks its per-chunk escape map; a cell stream
    /// fills the chunk's row/column. The default is the identity: a single-chunk
    /// source whose one chunk *is* the source, so a local location already
    /// addresses the source.
    ///
    /// Returns `None` when `local` has no source pre-image (out of bounds,
    /// inside an escape pair).
    ///
    /// [`M::Location`]: Modality::Location
    fn lift(&self, _chunk: &Chunk<M>, local: M::Location) -> Option<M::Location> {
        Some(local)
    }
}
