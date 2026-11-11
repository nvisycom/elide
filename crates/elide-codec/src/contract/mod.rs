//! The codec contract, grouped by concern:
//!
//! - `format`: *what kind of thing a codec is*. [`FormatId`], [`Format`]
//!   descriptor.
//! - `stream`: *one decoded modality stream*. [`Stream<M>`] (identify, chunk,
//!   flag, plus the inherited read/write and lift), with [`StreamMut`] /
//!   [`TypedStream`] the modality erasure a document lends a stream behind. The
//!   streamed unit is [`elide_core::modality::Chunk`].
//! - `loader`: *how raw bytes become a [`Document`]*. [`DocumentLoader`]
//!   produces the whole document; [`Loader`] is the per-modality leaf decoder,
//!   adapted by `LeafLoader`.
//! - `document`: *the decoded document*. A [`Document`] is its format's
//!   [`Native`] document, which owns the streams and writes the document out,
//!   plus the [`Blob`]s embedded in it; each is keyed by its [`LocalId`].
//!
//! The `FormatRegistry` that indexes these formats and the concrete streams
//! that implement them live in the assembly crates (`elide-format` and the
//! per-modality engines).

mod document;
mod format;
mod loader;
mod stream;
#[cfg(test)]
mod test_support;

pub use self::document::{
    Blob, Document, Encode, LeafDocument, LocalId, Native, Stripped, StrippedContent,
};
pub use self::format::{Format, FormatId};
pub(crate) use self::loader::LeafLoader;
pub use self::loader::{DocumentLoader, Loader};
pub use self::stream::{FlaggedItem, Stream, StreamMut, TypedStream};
