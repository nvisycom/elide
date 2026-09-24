//! The codec contract, grouped by concern:
//!
//! - `format`: *what kind of thing a codec is*. [`FormatId`], [`Format`]
//!   descriptor.
//! - `handler`: *what a handler exposes*. [`Handler<M>`] (per-modality
//!   capability surface: identify, encode, stream, plus the inherited
//!   read/write and lift). The streamed unit is [`elide_core::modality::Chunk`].
//! - `loader`: *how raw bytes become a handle*. [`Loader<M>`] (per-modality
//!   decoder). The registry-side modality-erasure machinery (`ErasedLoader`,
//!   `erase`) is crate-internal and wired through [`Format::new`] /
//!   [`Format::decode`].
//! - `document`: *the decoded handle*. [`DocumentHandle<M>`] (typed) and
//!   [`UntypedDocumentHandle`] (modality-erased, recovered by `TypeId`).
//! - `container`: *a document that nests sub-parts of other modalities*.
//!   [`Container`] exposing [`Part`]s.
//! - `local_id`: [`LocalId`], a container's own id for one of its parts.
//!
//! The `FormatRegistry` that indexes these formats and the concrete handlers
//! that implement them live in the assembly crates (`elide-format` and the
//! per-modality engines).

mod container;
mod document;
mod format;
mod handler;
pub(crate) mod loader;
mod local_id;

pub use self::container::{Container, Part};
pub use self::document::{DocumentHandle, UntypedDocumentHandle};
pub use self::format::{Format, FormatId};
pub use self::handler::Handler;
pub use self::loader::Loader;
pub use self::local_id::LocalId;
