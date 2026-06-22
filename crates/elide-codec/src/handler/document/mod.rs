//! Rich-document formats: DOCX, PDF, RTF.
//!
//! Some are *containers of parts across modalities*: a DOCX is a zip whose
//! body text lives in XML parts (`word/document.xml`, headers, footers)
//! and whose images live as separate files (`word/media/*`); a PDF holds
//! page text and image XObjects in its own object format. The container is
//! parsed, each text part is redacted (DOCX reuses the shared XML
//! [`extract`] engine), and the document is rebuilt with only those parts
//! changed — every other byte round-trips. Others, like RTF, are flat text
//! with no parts, handled as plain text handlers.
//!
//! Following Tika's recursive model, embedded media are not flattened
//! into the text stream; they are exposed as their own decodable handles
//! for the toolkit to drive through the image pipeline (lazy, opt-in —
//! see the media accessor).
//!
//! [`extract`]: crate::handler::extract

#[cfg(feature = "docx")]
mod docx_handler;
#[cfg(feature = "docx")]
mod docx_loader;
#[cfg(feature = "pdf")]
mod ocr_mode;
#[cfg(feature = "pdf")]
mod pdf_handler;
#[cfg(feature = "pdf")]
mod pdf_loader;
#[cfg(feature = "pdf-render")]
mod pdf_render;
#[cfg(feature = "rtf")]
mod rtf_handler;
#[cfg(feature = "rtf")]
mod rtf_loader;

#[cfg(feature = "docx")]
pub use self::docx_handler::format as docx_format;
#[cfg(feature = "docx")]
pub(crate) use self::docx_loader::DocxLoader;
#[cfg(feature = "pdf")]
pub use self::ocr_mode::OcrMode;
#[cfg(feature = "pdf")]
pub use self::pdf_handler::format as pdf_format;
#[cfg(feature = "pdf-render")]
pub use self::pdf_handler::format_with_ocr as pdf_format_with_ocr;
#[cfg(feature = "pdf")]
pub(crate) use self::pdf_loader::PdfLoader;
#[cfg(feature = "rtf")]
pub use self::rtf_handler::format as rtf_format;
#[cfg(feature = "rtf")]
pub(crate) use self::rtf_loader::RtfLoader;
