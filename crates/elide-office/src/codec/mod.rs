//! Codec adapters for Office documents: the DOCX, PPTX, and XLSX formats.
//!
//! `docx` and `pptx` are thin `OoxmlCodec` seams — a marker plus a `format()` —
//! over the shared element-text `ooxml` adapter; `xlsx` is its own codec
//! (shared-string cells are not the element-text path). Each names its
//! identity through `CodecIdentity` and builds its [`Format`] through
//! `format_of`. The `docProps/*` document-property parts every OOXML
//! container surfaces as metadata are handled once in `props`.

mod docx;
pub(crate) mod ooxml;
mod pptx;
pub(crate) mod props;
mod xlsx;

use elide_codec::{DocumentLoader, Format, FormatId, LocalId};

pub use self::docx::format as docx_format;
pub use self::pptx::format as pptx_format;
pub use self::xlsx::format as xlsx_format;
use crate::opc::PartPath;

/// An Office codec's identity: the stable [`FormatId`] it registers under, and
/// the file extension and MIME type it resolves by. The extension doubles as
/// the short lowercase name of the format in its error messages (`docx`).
pub(crate) trait CodecIdentity {
    /// The stable codec [`FormatId`].
    const FORMAT_ID: FormatId;

    /// The file extension the format resolves by, without the dot.
    const EXTENSION: &'static str;

    /// The MIME type the format resolves by.
    const CONTENT_TYPE: &'static str;
}

/// The [`Format`] of codec `C`: `loader`, resolved by `C`'s extension and MIME
/// type.
pub(crate) fn format_of<C: CodecIdentity>(loader: impl DocumentLoader) -> Format {
    Format::with_document_loader(C::FORMAT_ID, loader)
        .with_extensions([C::EXTENSION])
        .with_content_types([C::CONTENT_TYPE])
}

/// The package part a document-local id names: a property part or an
/// embedding is identified by its zip entry path.
pub(crate) fn part_path(id: &LocalId) -> PartPath {
    PartPath::from(id.as_str())
}

/// The document-local id of the package part at `path`: its zip entry path.
pub(crate) fn local_id(path: &PartPath) -> LocalId {
    LocalId::new(path.as_str().to_owned())
}
