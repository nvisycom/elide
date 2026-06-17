//! Concrete format handlers, grouped by modality.
//!
//! Each submodule ships per-format [`Loader`](crate::Loader) +
//! [`Handler`](crate::Handler) pairs and a `*_format()` constructor that
//! the [`CodecRegistry`](crate::CodecRegistry) registers. Submodules are
//! feature-gated; only the enabled formats are compiled and wired into
//! [`CodecRegistry::with_builtin`](crate::CodecRegistry::with_builtin).

#[cfg(any(feature = "txt", feature = "json", feature = "html", feature = "xml"))]
pub(crate) mod redact;

#[cfg(any(feature = "html", feature = "xml"))]
pub mod markup;
#[cfg(any(feature = "txt", feature = "json"))]
pub mod text;
