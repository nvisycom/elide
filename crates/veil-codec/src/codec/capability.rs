//! What a codec handler exposes — the trait surface every format
//! handler implements.
//!
//! - [`Handler<M>`] — per-modality capability trait. It *is* a
//!   [`DataReader`] + [`DataWriter`] (the random-access read / write
//!   surface, shared with the rest of the workspace), and adds the
//!   codec-specific bits on top: identify and serialise ([`format`],
//!   [`encode`]), stream chunks ([`next_chunk`]), and lift recognizer
//!   offsets back to source coordinates ([`lift_chunk`]).
//! - [`Chunk<M>`] — one decoded unit yielded by `next_chunk`.
//!
//! [`DataReader`]: veil_core::modality::DataReader
//! [`DataWriter`]: veil_core::modality::DataWriter
//! [`format`]: Handler::format
//! [`encode`]: Handler::encode
//! [`next_chunk`]: Handler::next_chunk
//! [`lift_chunk`]: Handler::lift_chunk

use std::future::Future;
use std::ops::Range;

use veil_core::Error;
use veil_core::modality::{DataReader, DataWriter, Modality};

use super::FormatId;
use crate::content::ContentData;

/// One decoded unit yielded by [`Handler::next_chunk`].
///
/// `data` is the per-modality wire payload; `location` is the coordinate
/// the handler accepts in [`read_at`] / [`write_at`] to address the same
/// chunk again. `hints` carries out-of-band context strings the chunk's
/// structural neighbours surface — CSV/XLSX column headers, JSON object
/// keys, HTML parent text — for context-aware recognizers; handlers
/// without such metadata leave it empty.
///
/// [`read_at`]: veil_core::modality::DataReader::read_at
/// [`write_at`]: veil_core::modality::DataWriter::write_at
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk<M: Modality> {
    /// Coordinate addressing this chunk inside the handler.
    pub location: M::Location,
    /// Wire payload at the chunk's location.
    pub data: M::Data,
    /// Out-of-band context strings recognizers should treat as
    /// in-context (column headers, parent element text, …). Empty when
    /// the handler has no such metadata to surface.
    pub hints: Vec<String>,
}

/// Per-modality capability trait every format handler implements.
///
/// A `Handler` *is* a [`DataReader`] + [`DataWriter`]: random-access read
/// (`read_at`) and batch redaction (`write_at`) come from those shared
/// traits, so a codec-backed document plugs straight into anything that
/// bounds on them (the toolkit's anonymizer, say). On top of that base,
/// `Handler` adds the codec-specific surface — identify and serialise
/// ([`format`], [`encode`]), stream chunks ([`next_chunk`]), and lift
/// recognizer offsets back to source coordinates ([`lift_chunk`]).
///
/// The handler owns the streaming cursor — concurrent iteration of the
/// same handle is not supported (only one `&mut self`).
///
/// Async methods return `impl Future` (RPITIT). The registry stores
/// handlers behind a crate-private object-safe bridge that boxes those
/// futures, so the public surface stays allocation-free for direct
/// callers.
///
/// [`DataReader`]: veil_core::modality::DataReader
/// [`DataWriter`]: veil_core::modality::DataWriter
/// [`format`]: Handler::format
/// [`encode`]: Handler::encode
/// [`next_chunk`]: Handler::next_chunk
/// [`lift_chunk`]: Handler::lift_chunk
pub trait Handler<M: Modality>: DataReader<M> + DataWriter<M> + Send + Sync + 'static {
    /// Stable id of the format this handler represents (e.g.
    /// `"veil.text.txt"`). Cheap to clone.
    fn format(&self) -> FormatId;

    /// Serialize the current handler content back to [`ContentData`].
    ///
    /// # Errors
    ///
    /// Returns an error when the in-memory representation cannot be
    /// re-encoded.
    fn encode(&self) -> Result<ContentData, Error>;

    /// Advance the cursor and yield the next chunk, or `None` at
    /// end-of-stream.
    fn next_chunk(&mut self) -> impl Future<Output = Result<Option<Chunk<M>>, Error>> + Send;

    /// Translate a `value_range` expressed inside `chunk.data`'s
    /// coordinate system into a source-coordinate `M::Location`.
    ///
    /// Recognizers see the decoded chunk payload and emit offsets into
    /// it; downstream stages need locations that address the handler's
    /// source bytes. For text-shaped handlers where `chunk.data` is a
    /// byte-for-byte slice of source, the mapping is the identity offset
    /// add against `chunk.location`. Handlers whose chunks decode
    /// escapes override to walk their per-chunk escape map.
    ///
    /// Returns `None` when the range has no source pre-image (out of
    /// bounds, inside an escape pair, or the modality has no meaningful
    /// `usize` value-range concept). The default is `None`.
    fn lift_chunk(&self, _chunk: &Chunk<M>, _value_range: Range<usize>) -> Option<M::Location> {
        None
    }
}
