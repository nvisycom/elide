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

#[cfg(feature = "internal_text")]
pub(crate) mod redact;

#[cfg(feature = "internal_audio")]
pub(crate) mod audio;
#[cfg(feature = "internal_document")]
pub(crate) mod document;
#[cfg(feature = "internal_extract")]
pub(crate) mod extract;
#[cfg(feature = "internal_image")]
pub(crate) mod image;
#[cfg(any(feature = "html", feature = "xml"))]
pub(crate) mod markup;
#[cfg(feature = "internal_tabular")]
pub(crate) mod tabular;
#[cfg(any(feature = "txt", feature = "json"))]
pub(crate) mod text;

// Public contract: the per-format constructors, plus the HTML
// script-handling config its `format_with` constructor takes.
#[cfg(feature = "mp3")]
#[cfg_attr(docsrs, doc(cfg(feature = "mp3")))]
pub use self::audio::mp3_format;
#[cfg(feature = "wav")]
#[cfg_attr(docsrs, doc(cfg(feature = "wav")))]
pub use self::audio::wav_format;
#[cfg(feature = "docx")]
#[cfg_attr(docsrs, doc(cfg(feature = "docx")))]
pub use self::document::docx_format;
#[cfg(feature = "pdf-render")]
#[cfg_attr(docsrs, doc(cfg(feature = "pdf-render")))]
pub use self::document::pdf_format_with;
#[cfg(feature = "rtf")]
#[cfg_attr(docsrs, doc(cfg(feature = "rtf")))]
pub use self::document::rtf_format;
#[cfg(feature = "pdf")]
#[cfg_attr(docsrs, doc(cfg(feature = "pdf")))]
pub use self::document::{OcrMode, pdf_format};
#[cfg(feature = "jpeg")]
#[cfg_attr(docsrs, doc(cfg(feature = "jpeg")))]
pub use self::image::jpeg_format;
#[cfg(feature = "png")]
#[cfg_attr(docsrs, doc(cfg(feature = "png")))]
pub use self::image::png_format;
#[cfg(feature = "tiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "tiff")))]
pub use self::image::tiff_format;
#[cfg(feature = "xml")]
#[cfg_attr(docsrs, doc(cfg(feature = "xml")))]
pub use self::markup::xml_format;
#[cfg(feature = "html")]
#[cfg_attr(docsrs, doc(cfg(feature = "html")))]
pub use self::markup::{ScriptPolicy, html_format, html_format_with};
#[cfg(feature = "csv")]
#[cfg_attr(docsrs, doc(cfg(feature = "csv")))]
pub use self::tabular::csv_format;
#[cfg(feature = "xlsx")]
#[cfg_attr(docsrs, doc(cfg(feature = "xlsx")))]
pub use self::tabular::xlsx_format;
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub use self::text::json_format;
#[cfg(feature = "txt")]
#[cfg_attr(docsrs, doc(cfg(feature = "txt")))]
pub use self::text::txt_format;
