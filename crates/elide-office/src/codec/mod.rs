//! Codec adapters: the OOXML formats (DOCX, PPTX, XLSX).
//!
//! `docx` and `pptx` are thin `OoxmlCodec` seams — a marker plus a `format()` —
//! over the shared element-text `ooxml` adapter; `xlsx` is its own codec
//! (shared-string cells are not the element-text path). The `docProps/*`
//! document-property parts every OOXML container surfaces as metadata are
//! handled once in `props`, and the OPC source-span helpers in `opc_source`.

mod docx;
pub(crate) mod ooxml;
mod opc_source;
mod pptx;
pub(crate) mod props;
mod xlsx;

pub use self::docx::format as docx_format;
pub use self::pptx::format as pptx_format;
pub use self::xlsx::format as xlsx_format;
