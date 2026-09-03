//! The codec contracts, grouped by concern:
//!
//! - `format`: *what kind of thing a codec is*. [`FormatId`],
//!   [`Format`] descriptor.
//! - `handler`: *what a handler exposes*. [`Handler<M>`]
//!   (per-modality capability surface: identify, encode, stream, plus
//!   the inherited read/write and lift). The streamed unit is
//!   [`elide_core::modality::Chunk`].
//! - `loader`: *how raw bytes become a handle*. [`Loader<M>`]
//!   (per-modality decoder). The registry-side modality-erasure machinery
//!   (`ErasedLoader`, `erase`) is crate-internal and wired through
//!   [`Format::new`] / [`Format::decode`].
//! - `document`: *the decoded handle*. [`DocumentHandle<M>`] (typed) and
//!   [`UntypedDocumentHandle`] (modality-erased, recovered by `TypeId`).
//! - `registry`: *the lookup engine*. [`FormatRegistry`] indexes
//!   [`Format`]s by id, extension, and content type, and decodes bytes
//!   through the matching loader.
//!
//! Concrete format implementations live in `crate::handler::*`.

mod container;
mod document;
mod format;
mod handler;
pub(crate) mod loader;
mod local_id;
mod registry;

pub use self::container::{Container, Part};
pub use self::document::{DocumentHandle, UntypedDocumentHandle};
pub use self::format::{Format, FormatId};
pub use self::handler::Handler;
pub use self::loader::Loader;
pub use self::local_id::LocalId;
pub use self::registry::FormatRegistry;
