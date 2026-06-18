//! What a codec handler exposes: the trait surface every format
//! handler implements.
//!
//! - [`Handler<M>`]: per-modality capability trait. It *is* a
//!   [`DataReader`] + [`DataWriter`] (the random-access read / write
//!   surface, shared with the rest of the workspace), and adds the
//!   codec-specific bits on top: identify and serialise ([`format`],
//!   [`encode`]), stream chunks ([`read_next`]), and lift recognizer
//!   offsets back to source coordinates ([`lift_chunk`]).
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
//! [`lift_chunk`]: Handler::lift_chunk

use std::future::Future;
use std::ops::Range;

use elide_core::Result;
use elide_core::modality::{Chunk, DataReader, DataWriter, Modality};

use super::FormatId;
use crate::content::ContentData;

/// Per-modality capability trait every format handler implements.
///
/// A `Handler` *is* a [`DataReader`] + [`DataWriter`]: random-access read
/// (`read_at`) and batch redaction (`write_at`) come from those shared
/// traits, so a codec-backed document plugs straight into anything that
/// bounds on them (the toolkit's anonymizer, say). On top of that base,
/// `Handler` adds the codec-specific surface: identify and serialise
/// ([`format`], [`encode`]), stream chunks ([`read_next`]), and lift
/// recognizer offsets back to source coordinates ([`lift_chunk`]).
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
/// [`lift_chunk`]: Handler::lift_chunk
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
