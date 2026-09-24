//! Codec adapters: the OOXML format handlers (DOCX, PPTX, XLSX).
//!
//! They adapt this crate's package engine to the `elide-codec`
//! `Handler`/`Loader` contracts, plus the shared `docProps/*`
//! document-property sub-part every OOXML container surfaces.
//!
//! The three container formats decode to the same [`crate::ooxml::OoxmlPackage`]
//! (docx, pptx) or [`crate::xlsx::Xlsx`] engine and adapt the same way; the
//! shared machinery, the error mapping, the OPC part-addressing helpers, and
//! the document-property handler, lives in this module once.

mod docx_handler;
mod docx_loader;
mod ooxml;
mod opc_source;
mod pptx_handler;
mod pptx_loader;
mod props;
mod xlsx_handler;
mod xlsx_loader;

pub use self::docx_handler::format as docx_format;
pub(crate) use self::docx_loader::DocxLoader;
pub use self::pptx_handler::format as pptx_format;
pub(crate) use self::pptx_loader::PptxLoader;
pub use self::props::{DocPropsRecognizer, format as docprops_format};
pub use self::xlsx_handler::format as xlsx_format;

/// The registry hint (pseudo-extension) the OOXML `#docprops` sub-part is decoded
/// with; every OOXML container surfaces its property parts under it.
pub(crate) fn docprops_hint() -> &'static str {
    self::props::PROPS_HINT
}

/// Map an [`elide_office`](crate) error into the codec's error type.
///
/// A malformed package (bad zip, missing part, invalid XML) is the caller's
/// [`MalformedInput`](elide_core::ErrorKind::MalformedInput); everything else,
/// an unsafe rewrite or any future engine error, is a
/// [`Processing`](elide_core::ErrorKind::Processing) failure. The wildcard is
/// load-bearing: `crate::ErrorKind` is `#[non_exhaustive]`, and a new variant
/// defaults to `Processing`.
pub(crate) fn office_error(err: crate::Error) -> elide_core::Error {
    use elide_core::{Error, ErrorKind};

    use crate::ErrorKind::{InvalidArchive, InvalidPackage, InvalidXml};
    let kind = match err.kind() {
        InvalidArchive | InvalidPackage | InvalidXml => ErrorKind::MalformedInput,
        _ => ErrorKind::Processing,
    };
    Error::new(kind, err.to_string())
}
