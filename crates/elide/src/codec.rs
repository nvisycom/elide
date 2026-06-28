//! Codec: decode documents into modality payloads, then re-encode them.
//!
//! Format handlers (text, JSON, HTML, images, audio, …) sit behind a
//! [`FormatRegistry`]: each turns raw bytes into something recognizers
//! and operators can address, then folds the redactions back into the
//! original container. Re-exported from [`elide_codec`].
//!
//! [`FormatRegistry`]: elide_codec::FormatRegistry

// The glob brings the `content` and `handler` submodules along with the
// registry and handle types.
#[doc(inline)]
pub use elide_codec::*;
