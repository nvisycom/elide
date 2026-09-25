//! The codec contract, grouped by concern:
//!
//! - `format`: *what kind of thing a codec is*. [`FormatId`], [`Format`]
//!   descriptor.
//! - `stream`: *one decoded modality stream*. [`Stream<M>`] (identify, encode,
//!   stream, plus the inherited read/write and lift), the per-part surface of a
//!   [`Document`], with [`ErasedStream`] / [`TypedStream`] the modality erasure a
//!   stream part is stored behind. The streamed unit is
//!   [`elide_core::modality::Chunk`].
//! - `loader`: *how raw bytes become a [`Document`]*. [`DocumentLoader`]
//!   produces the whole document (stream parts, blob sub-parts, recombiner);
//!   [`Loader`] is the per-modality leaf decoder, adapted by [`LeafLoader`].
//! - `document`: *the decoded document*. [`Document`] is a `Vec` of
//!   [`DocumentPart`]s (a [`Stream`] or a [`Blob`](DocumentPart::Blob)),
//!   recomposed by its [`Recombine`]; a part is keyed by its [`LocalId`].
//!
//! The `FormatRegistry` that indexes these formats and the concrete streams
//! that implement them live in the assembly crates (`elide-format` and the
//! per-modality engines).

mod document;
mod format;
pub(crate) mod loader;
mod stream;
#[cfg(test)]
mod test_support;

pub use self::document::{Document, DocumentPart, EncodedPart, LeafRecombine, LocalId, Recombine};
pub use self::format::{Format, FormatId};
pub use self::loader::{DocumentLoader, LeafLoader, Loader};
pub use self::stream::{ErasedStream, Stream, TypedStream};
