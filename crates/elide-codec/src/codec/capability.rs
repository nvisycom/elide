//! What a codec handler exposes: the trait surface every format
//! handler implements.
//!
//! - [`Handler<M>`]: per-modality capability trait. It *is* a
//!   [`DataReader`] + [`DataWriter`] (the random-access read / write
//!   surface, shared with the rest of the workspace), and adds the
//!   codec-specific bits on top: identify and serialise ([`format`],
//!   [`encode`]), stream chunks ([`read_next`]), and lift a chunk-local
//!   finding back to source coordinates ([`lift`]).
//!
//! [`Chunk<M>`], the unit yielded by `read_next`, lives in
//! [`elide_core::modality`], since it is the shared currency of the
//! [`StreamDataReader`] contract, not a codec-private type.
//!
//! [`DataReader`]: elide_core::modality::DataReader
//! [`DataWriter`]: elide_core::modality::DataWriter
//! [`StreamDataReader`]: elide_core::modality::StreamDataReader
//! [`Chunk<M>`]: elide_core::modality::Chunk
//! [`format`]: Handler::format
//! [`encode`]: Handler::encode
//! [`read_next`]: Handler::read_next
//! [`lift`]: Handler::lift

use std::future::Future;

use elide_core::Result;
use elide_core::modality::{Chunk, DataReader, DataWriter, Modality};

use super::{Container, FormatId};
use crate::content::ContentData;

/// Per-modality capability trait every format handler implements.
///
/// A `Handler` *is* a [`DataReader`] + [`DataWriter`]: random-access read
/// (`read_at`) and batch redaction (`write_at`) come from those shared
/// traits, so a codec-backed document plugs straight into anything that
/// bounds on them (the toolkit's anonymizer, say). On top of that base,
/// `Handler` adds the codec-specific surface: identify and serialise
/// ([`format`], [`encode`]), stream chunks ([`read_next`]), and lift a
/// chunk-local finding back to source coordinates ([`lift`]).
///
/// The handler owns the streaming cursor; concurrent iteration of the
/// same handle is not supported (only one `&mut self`).
///
/// Async methods return `impl Future` (RPITIT). The registry stores
/// handlers behind a crate-private object-safe bridge that boxes those
/// futures, so the public surface stays allocation-free for direct
/// callers.
///
/// [`DataReader`]: elide_core::modality::DataReader
/// [`DataWriter`]: elide_core::modality::DataWriter
/// [`format`]: Handler::format
/// [`encode`]: Handler::encode
/// [`read_next`]: Handler::read_next
/// [`lift`]: Handler::lift
pub trait Handler<M: Modality>: DataReader<M> + DataWriter<M> + Send + Sync + 'static {
    /// Stable id of the format this handler represents (e.g.
    /// `"elide.text.txt"`). Cheap to clone.
    fn format(&self) -> FormatId;

    /// Serialize the current handler content back to [`ContentData`].
    ///
    /// # Errors
    ///
    /// Returns an error when the in-memory representation cannot be
    /// re-encoded.
    fn encode(&self) -> Result<ContentData>;

    /// Advance the cursor and yield the next chunk, or `None` at
    /// end-of-stream.
    fn read_next(&mut self) -> impl Future<Output = Result<Option<Chunk<M>>>> + Send;

    /// Promote a `local` location, expressed in `chunk`'s own coordinate
    /// system, to a source-global [`M::Location`].
    ///
    /// A recognizer sees a chunk's decoded payload and emits a finding in
    /// *chunk-local* coordinates: a byte range into the chunk text, a box
    /// within the chunk frame, a span within the chunk's clip. Downstream
    /// stages need locations that address the whole source, so the handler
    /// rebases the local one onto the chunk's origin.
    ///
    /// For a chunk that is a byte-for-byte slice of source the mapping is
    /// the identity offset add against `chunk.location`; a handler whose
    /// chunks decode escapes (JSON) walks its per-chunk escape map; a cell
    /// handler fills the chunk's row/column. The default is the identity:
    /// a single-chunk source whose one chunk *is* the source, so a local
    /// location already addresses the source.
    ///
    /// Returns `None` when `local` has no source pre-image (out of bounds,
    /// inside an escape pair).
    ///
    /// [`M::Location`]: Modality::Location
    fn lift(&self, _chunk: &Chunk<M>, local: M::Location) -> Option<M::Location> {
        Some(local)
    }

    /// This handler as a [`Container`] of cross-modality sub-parts, if it
    /// is one (DOCX, ahead PDF). The default is `None`: a plain
    /// single-modality format is not a container.
    ///
    /// [`Container`]: crate::Container
    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        None
    }
}
