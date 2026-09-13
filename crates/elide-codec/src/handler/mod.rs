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

#[cfg(any(feature = "json", feature = "xml", feature = "html"))]
pub(crate) mod context;

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
#[cfg(feature = "internal_office")]
pub(crate) mod office;
#[cfg(feature = "internal_tabular")]
pub(crate) mod tabular;
#[cfg(any(feature = "txt", feature = "json"))]
pub(crate) mod text;

// Public contract: the per-format constructors, plus the HTML
// script-handling config its `format_with` constructor takes.
/// The [`ExifPolicy`] an image `format_with` takes as its no-pipeline metadata
/// fallback — a dependency-free config value, gated only on the image engine
/// being present (`internal_image`), not on the metadata recognizer.
#[cfg(feature = "internal_image")]
#[cfg_attr(docsrs, doc(cfg(feature = "internal_image")))]
pub use ::elide_image::ExifPolicy;
/// The recognizer that classifies an image's EXIF fields into
/// `Entity<Metadata>` values, for a caller wiring the metadata pipeline.
#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "png", feature = "jpeg", feature = "tiff")))
)]
pub use ::elide_image::ExifRecognizer;

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
#[cfg(feature = "pptx")]
#[cfg_attr(docsrs, doc(cfg(feature = "pptx")))]
pub use self::document::pptx_format;
#[cfg(feature = "rtf")]
#[cfg_attr(docsrs, doc(cfg(feature = "rtf")))]
pub use self::document::rtf_format;
#[cfg(feature = "pdf")]
#[cfg_attr(docsrs, doc(cfg(feature = "pdf")))]
pub use self::document::{RasterMode, pdf_format};
#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "png", feature = "jpeg", feature = "tiff")))
)]
pub use self::image::exif_format;
#[cfg(feature = "jpeg")]
#[cfg_attr(docsrs, doc(cfg(feature = "jpeg")))]
pub use self::image::{jpeg_format, jpeg_format_with};
#[cfg(feature = "png")]
#[cfg_attr(docsrs, doc(cfg(feature = "png")))]
pub use self::image::{png_format, png_format_with};
#[cfg(feature = "tiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "tiff")))]
pub use self::image::{tiff_format, tiff_format_with};
#[cfg(feature = "xml")]
#[cfg_attr(docsrs, doc(cfg(feature = "xml")))]
pub use self::markup::xml_format;
#[cfg(feature = "html")]
#[cfg_attr(docsrs, doc(cfg(feature = "html")))]
pub use self::markup::{ScriptPolicy, html_format, html_format_with};
#[cfg(feature = "internal_office")]
pub(crate) use self::office::docprops_hint;
/// The OOXML document-property (`docProps/*`) metadata format + its recognizer,
/// for a caller wiring the metadata pipeline to strip document properties from
/// docx/pptx/xlsx.
#[cfg(feature = "internal_office")]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "docx", feature = "pptx", feature = "xlsx")))
)]
pub use self::office::{DocPropsRecognizer, docprops_format};
#[cfg(feature = "xlsx")]
#[cfg_attr(docsrs, doc(cfg(feature = "xlsx")))]
pub use self::tabular::xlsx_format;
#[cfg(feature = "csv")]
#[cfg_attr(docsrs, doc(cfg(feature = "csv")))]
pub use self::tabular::{csv_format, csv_format_with};
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub use self::text::json_format;
#[cfg(feature = "txt")]
#[cfg_attr(docsrs, doc(cfg(feature = "txt")))]
pub use self::text::txt_format;
