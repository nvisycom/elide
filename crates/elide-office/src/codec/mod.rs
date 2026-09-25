//! Codec adapters: the OOXML formats (DOCX, PPTX, XLSX) on the parts model.
//!
//! [`docx`] and [`pptx`] are thin [`OoxmlCodec`](ooxml::OoxmlCodec) seams — a
//! marker plus a `format()` — over the shared element-text [`ooxml`] adapter;
//! [`xlsx`] is its own codec (shared-string cells are not the element-text path).
//! The shared `docProps/*` document-property sub-part every OOXML container
//! surfaces is handled once in [`props`], and the OPC source-span helpers in
//! [`opc_source`].

mod docx;
pub(crate) mod ooxml;
mod opc_source;
mod pptx;
pub(crate) mod props;
mod xlsx;

pub use self::docx::format as docx_format;
pub use self::pptx::format as pptx_format;
pub use self::props::{DocPropsRecognizer, format as docprops_format};
pub use self::xlsx::format as xlsx_format;

/// The registry hint (pseudo-extension) the OOXML `#docprops` sub-part is decoded
/// with; every OOXML container surfaces its property parts under it.
pub(crate) fn docprops_hint() -> &'static str {
    self::props::PROPS_HINT
}
