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

/// PDF codec: the [`pdf_format`]/[`pdf_format_with`] constructors, plus the
/// [`RasterMode`] and [`Dpi`] configuration a PDF format is built with.
///
/// Pass a constructor to [`FormatRegistry::with_replaced_format`] to swap the
/// registered handler for a specific format (e.g. the raster path).
///
/// [`pdf_format`]: elide_pdf::codec::pdf_format
/// [`pdf_format_with`]: elide_pdf::codec::pdf_format_with
/// [`RasterMode`]: elide_pdf::primitive::RasterMode
/// [`Dpi`]: elide_pdf::primitive::Dpi
/// [`FormatRegistry::with_replaced_format`]: elide_format::FormatRegistry::with_replaced_format
#[cfg(feature = "codec-pdf")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec-pdf")))]
pub mod pdf {
    #[doc(inline)]
    pub use elide_pdf::codec::pdf_format;
    #[cfg(feature = "codec-pdf-render")]
    #[doc(inline)]
    pub use elide_pdf::codec::pdf_format_with;
    #[doc(inline)]
    pub use elide_pdf::primitive::{Dpi, RasterMode};
}

/// Raster-image codec: the `*_format`/`*_format_with` constructors for the PNG,
/// JPEG, and TIFF handlers (plus [`exif_format`] for the standalone
/// EXIF-metadata format), and the [`ExifPolicy`] a `*_format_with` is built with.
///
/// Pass a constructor to [`FormatRegistry::with_replaced_format`] to swap the
/// registered handler for a specific format (e.g. a stricter EXIF policy).
///
/// [`exif_format`]: elide_image::codec::exif_format
/// [`ExifPolicy`]: elide_image::exif::ExifPolicy
/// [`FormatRegistry::with_replaced_format`]: elide_format::FormatRegistry::with_replaced_format
#[cfg(any(feature = "codec-png", feature = "codec-jpeg", feature = "codec-tiff"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "codec-png", feature = "codec-jpeg", feature = "codec-tiff")))
)]
pub mod image {
    #[doc(inline)]
    pub use elide_image::codec::exif_format;
    #[cfg(feature = "codec-jpeg")]
    #[doc(inline)]
    pub use elide_image::codec::{jpeg_format, jpeg_format_with};
    #[cfg(feature = "codec-png")]
    #[doc(inline)]
    pub use elide_image::codec::{png_format, png_format_with};
    #[cfg(feature = "codec-tiff")]
    #[doc(inline)]
    pub use elide_image::codec::{tiff_format, tiff_format_with};
    #[doc(inline)]
    pub use elide_image::exif::ExifPolicy;
}

/// Plain-text codec: the `*_format`/`*_format_with` constructors for the
/// text-shaped formats (TXT, JSON, HTML, XML, CSV), and the [`ScriptPolicy`] an
/// HTML format is built with.
///
/// Pass a constructor to [`FormatRegistry::with_replaced_format`] to swap the
/// registered handler for a specific format (e.g. HTML that scans `<script>`
/// bodies, or a custom CSV dialect).
///
/// [`ScriptPolicy`]: elide_plain::primitive::ScriptPolicy
/// [`FormatRegistry::with_replaced_format`]: elide_format::FormatRegistry::with_replaced_format
#[cfg(any(
    feature = "codec-txt",
    feature = "codec-json",
    feature = "codec-html",
    feature = "codec-xml",
    feature = "codec-csv"
))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(
        feature = "codec-txt",
        feature = "codec-json",
        feature = "codec-html",
        feature = "codec-xml",
        feature = "codec-csv"
    )))
)]
pub mod plain {
    #[cfg(feature = "codec-json")]
    #[doc(inline)]
    pub use elide_plain::json_format;
    #[cfg(feature = "codec-html")]
    #[doc(inline)]
    pub use elide_plain::primitive::ScriptPolicy;
    #[cfg(feature = "codec-txt")]
    #[doc(inline)]
    pub use elide_plain::txt_format;
    #[cfg(feature = "codec-xml")]
    #[doc(inline)]
    pub use elide_plain::xml_format;
    #[cfg(feature = "codec-csv")]
    #[doc(inline)]
    pub use elide_plain::{csv_format, csv_format_with};
    #[cfg(feature = "codec-html")]
    #[doc(inline)]
    pub use elide_plain::{html_format, html_format_with};
}

/// Office-document codec: the `*_format` constructors for the OOXML containers
/// (DOCX, PPTX, XLSX) and the shared `docProps` metadata sub-part. No format
/// takes configuration.
///
/// Pass a constructor to [`FormatRegistry::with_replaced_format`] to swap the
/// registered handler for a specific format. The matching [`DocPropsRecognizer`]
/// lives under [`recognition::docprops`](crate::recognition::docprops).
///
/// [`DocPropsRecognizer`]: crate::recognition::docprops::DocPropsRecognizer
/// [`FormatRegistry::with_replaced_format`]: elide_format::FormatRegistry::with_replaced_format
#[cfg(any(feature = "codec-docx", feature = "codec-pptx", feature = "codec-xlsx"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "codec-docx", feature = "codec-pptx", feature = "codec-xlsx")))
)]
pub mod office {
    #[doc(inline)]
    pub use elide_office::codec::docprops_format;
    #[cfg(feature = "codec-docx")]
    #[doc(inline)]
    pub use elide_office::codec::docx_format;
    #[cfg(feature = "codec-pptx")]
    #[doc(inline)]
    pub use elide_office::codec::pptx_format;
    #[cfg(feature = "codec-xlsx")]
    #[doc(inline)]
    pub use elide_office::codec::xlsx_format;
}
