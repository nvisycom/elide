//! Codec: decode documents into modality payloads, then re-encode them.
//!
//! Format handlers (text, JSON, HTML, images, audio, …) sit behind a
//! [`FormatRegistry`]: each turns raw bytes into something recognizers
//! and operators can address, then folds the redactions back into the
//! original container. The codec *contracts* come from [`elide_codec`]; the
//! [`FormatRegistry`] that assembles the handlers comes from [`elide_format`].
//!
//! [`FormatRegistry`]: elide_format::FormatRegistry

// The glob brings the `content` submodule along with the trait and handle
// types; the registry comes from the assembly crate.
#[doc(inline)]
pub use elide_codec::*;
#[doc(inline)]
pub use elide_format::FormatRegistry;

/// PDF codec configuration: the [`RasterMode`] and [`Dpi`] a PDF format is
/// built with, plus the [`pdf_format`]/[`pdf_format_with`] constructors for
/// swapping the registered handler (e.g. the raster path).
///
/// [`RasterMode`]: elide_pdf::primitive::RasterMode
/// [`Dpi`]: elide_pdf::primitive::Dpi
/// [`pdf_format`]: elide_pdf::codec::pdf_format
/// [`pdf_format_with`]: elide_pdf::codec::pdf_format_with
#[cfg(feature = "codec-pdf")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec-pdf")))]
pub mod pdf {
    #[doc(no_inline)]
    pub use elide_pdf::codec::pdf_format;
    #[cfg(feature = "codec-pdf-render")]
    #[doc(no_inline)]
    pub use elide_pdf::codec::pdf_format_with;
    #[doc(no_inline)]
    pub use elide_pdf::primitive::{Dpi, RasterMode};
}
