//! [`Stream<M>`]: a redactable modality stream, one of a document's streams.
//!
//! A `Stream` is the read/redact surface for one modality's content:
//! a text body, an image's pixels, an audio clip. It is a [`DataReader`] +
//! [`DataWriter`] (random-access read / batch redact, shared with the rest of
//! the workspace) plus the codec-specific stream surface — identify, chunk,
//! and lift. A document's embedded files are not streams; they are the
//! [`Document`](super::Document)'s [`Blob`](super::Blob)s.
//!
//! [`erased`] holds [`StreamMut`] / [`TypedStream`], the modality erasure a
//! document lends its streams of different modalities behind.
//!
//! [`DataReader`]: elide_core::modality::DataReader
//! [`DataWriter`]: elide_core::modality::DataWriter

mod erased;
mod flagged;

use std::any::Any;

use elide_core::Result;
use elide_core::modality::{Chunk, DataReader, DataWriter, Modality};

pub use self::erased::{StreamMut, TypedStream};
pub use self::flagged::FlaggedItem;
use super::FormatId;

/// A redactable modality stream, owned by its format's [`Native`] document.
///
/// A `Stream` *is* a [`DataReader`] + [`DataWriter`]: random-access read
/// (`read_at`) and batch redaction (`write_at`) come from those shared traits,
/// so a codec-backed stream plugs straight into anything that bounds on them
/// (the toolkit's anonymizer). On top of that base it adds the codec surface:
/// identify itself ([`format`]), hand back its decoded chunks ([`chunks`]) and
/// the hidden content its format labels ([`flagged`]), and lift a chunk-local
/// finding back to source coordinates ([`lift`]).
///
/// A stream does not write bytes: the [`Native`] document that owns it reads it
/// and writes the document out. It is lent to the engine as a
/// [`StreamMut`], erased through the `Any` supertrait.
///
/// A stream's chunks are materialized when it is decoded, so [`chunks`] is a
/// plain `&self` read; only redaction ([`write_at`]) mutates.
///
/// [`DataReader`]: elide_core::modality::DataReader
/// [`DataWriter`]: elide_core::modality::DataWriter
/// [`write_at`]: elide_core::modality::DataWriter::write_at
/// [`Native`]: super::Native
/// [`format`]: Stream::format
/// [`chunks`]: Stream::chunks
/// [`flagged`]: Stream::flagged
/// [`lift`]: Stream::lift
pub trait Stream<M: Modality>: DataReader<M> + DataWriter<M> + Any + Send + Sync {
    /// Stable id of the format this stream represents (e.g. `"elide.text.txt"`).
    fn format(&self) -> FormatId;

    /// Every decoded chunk this stream holds, front to back.
    ///
    /// # Errors
    ///
    /// Returns the source's decode error.
    fn chunks(&self) -> Result<Vec<Chunk<M>>>;

    /// The items of hidden content this stream keeps and its format labels
    /// (metadata fields, comments, their authors, tracked deletions), front to
    /// back. Empty by default: a stream of visible content flags nothing.
    ///
    /// # Errors
    ///
    /// Returns the source's decode error.
    fn flagged(&self) -> Result<Vec<FlaggedItem<M>>> {
        Ok(Vec::new())
    }

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
