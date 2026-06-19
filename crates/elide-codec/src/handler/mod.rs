//! Concrete format handlers, grouped by modality.
//!
//! Each submodule ships per-format [`Loader`] + [`Handler`] pairs behind
//! a `*_format()` constructor. Those constructors are the module's public
//! surface; the registry wires them into [`FormatRegistry::with_builtin`],
//! and they are re-exported here so callers reach them as
//! `handler::txt_format()` rather than through the crate-internal
//! submodules. The concrete loaders, handlers, and encoders stay private.
//! Submodules are feature-gated; only the enabled formats are compiled.
//!
//! [`Loader`]: crate::Loader
//! [`Handler`]: crate::Handler
//! [`FormatRegistry::with_builtin`]: crate::FormatRegistry::with_builtin

#[cfg(any(feature = "txt", feature = "json", feature = "html", feature = "xml"))]
pub(crate) mod redact;

#[cfg(any(feature = "html", feature = "xml"))]
pub(crate) mod markup;
#[cfg(any(feature = "txt", feature = "json"))]
pub(crate) mod text;

// Public contract: the per-format constructors, plus the HTML
// script-handling config its `format_with` constructor takes.
#[cfg(feature = "xml")]
#[cfg_attr(docsrs, doc(cfg(feature = "xml")))]
pub use self::markup::xml_format;
#[cfg(feature = "html")]
#[cfg_attr(docsrs, doc(cfg(feature = "html")))]
pub use self::markup::{ScriptPolicy, html_format, html_format_with};
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub use self::text::json_format;
#[cfg(feature = "txt")]
#[cfg_attr(docsrs, doc(cfg(feature = "txt")))]
pub use self::text::txt_format;
