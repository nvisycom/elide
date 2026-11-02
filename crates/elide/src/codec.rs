//! Codec: decode documents into modality payloads, then re-encode them.
//!
//! Format handlers (text, JSON, HTML, images, audio, …) sit behind a
//! [`FormatRegistry`]: each turns raw bytes into something recognizers
//! and operators can address, then folds the redactions back into the
//! original container. The codec *contracts* come from [`elide_codec`]; the
//! [`FormatRegistry`] that assembles the handlers comes from [`elide_format`].
//!
//! Every decode takes a [`HiddenContentPolicy`]: hidden content (metadata,
//! comments and revisions, previews, scripts, external references) is
//! stripped before detection runs, every document embedded in the decoded one
//! is handled under the same policy, and every encode checks the output still
//! carries none.
//!
//! [`FormatRegistry`]: elide_format::FormatRegistry
//! [`HiddenContentPolicy`]: crate::redaction::HiddenContentPolicy

// The codec contract: the document/stream types and the loader/recombine
// traits a handler implements, plus the `ContentData`/`TextEncoding` its
// `decode`/`encode` signatures speak in. The registry that assembles the
// handlers comes from the assembly crate. The crate's internal helpers
// (`string`, `extract`) and test doubles (`mocks`) are deliberately not
// re-exported here.
#[doc(inline)]
pub use elide_codec::content::{ContentData, TextEncoding};
#[doc(inline)]
pub use elide_codec::{
    Document, DocumentLoader, DocumentPart, EncodedPart, ErasedStream, Format, FormatId,
    LeafLoader, LeafRecombine, Loader, LocalId, Recombine, Stream, Stripped, StrippedContent,
    TypedStream,
};
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

/// Raster-image codec: the `*_format` constructors for the PNG, JPEG, and TIFF
/// handlers, plus [`exif_format`] for the EXIF-metadata sub-part a raster
/// image surfaces when its metadata is redacted.
///
/// Pass a constructor to [`FormatRegistry::with_replaced_format`] to swap the
/// registered handler for a specific format.
///
/// [`exif_format`]: elide_image::codec::exif_format
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
    pub use elide_image::codec::jpeg_format;
    #[cfg(feature = "codec-png")]
    #[doc(inline)]
    pub use elide_image::codec::png_format;
    #[cfg(feature = "codec-tiff")]
    #[doc(inline)]
    pub use elide_image::codec::tiff_format;
}

/// Plain-text codec: the `*_format` constructors for the plain-text formats
/// (TXT, JSON, CSV), and [`csv_format_with`] for a custom CSV dialect.
///
/// Pass a constructor to [`FormatRegistry::with_replaced_format`] to swap the
/// registered handler for a specific format.
///
/// [`csv_format_with`]: elide_plain::csv_format_with
/// [`FormatRegistry::with_replaced_format`]: elide_format::FormatRegistry::with_replaced_format
#[cfg(any(feature = "codec-txt", feature = "codec-json", feature = "codec-csv"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "codec-txt", feature = "codec-json", feature = "codec-csv")))
)]
pub mod plain {
    #[cfg(feature = "codec-json")]
    #[doc(inline)]
    pub use elide_plain::json_format;
    #[cfg(feature = "codec-txt")]
    #[doc(inline)]
    pub use elide_plain::txt_format;
    #[cfg(feature = "codec-csv")]
    #[doc(inline)]
    pub use elide_plain::{csv_format, csv_format_with};
}

/// Markup codec: the `*_format` constructors for the HTML and XML handlers.
///
/// Pass a constructor to [`FormatRegistry::with_replaced_format`] to swap the
/// registered handler for a specific format.
///
/// [`FormatRegistry::with_replaced_format`]: elide_format::FormatRegistry::with_replaced_format
#[cfg(any(feature = "codec-html", feature = "codec-xml"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "codec-html", feature = "codec-xml"))))]
pub mod markup {
    #[cfg(feature = "codec-html")]
    #[doc(inline)]
    pub use elide_markup::html_format;
    #[cfg(feature = "codec-xml")]
    #[doc(inline)]
    pub use elide_markup::xml_format;
}

/// Office-document codec: the `*_format` constructors for the OOXML containers
/// (DOCX, PPTX, XLSX) and the shared `docProps` metadata sub-part. No format
/// takes configuration.
///
/// Pass a constructor to [`FormatRegistry::with_replaced_format`] to swap the
/// registered handler for a specific format. Each `docProps` field it surfaces
/// is labelled, so [`recognition::metadata::FieldRecognizer`] flags it.
///
/// [`recognition::metadata::FieldRecognizer`]: crate::recognition::metadata::FieldRecognizer
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
