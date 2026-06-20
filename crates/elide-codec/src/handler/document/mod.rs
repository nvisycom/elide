//! Rich-document container formats (DOCX, …) over the shared extract
//! engine.
//!
//! A rich document is a *container of parts across modalities*: a DOCX is
//! a zip whose body text lives in XML parts (`word/document.xml`,
//! headers, footers) and whose images live as separate files
//! (`word/media/*`). The container is unzipped, each text part is
//! redacted through the shared XML [`extract`] engine, and the package is
//! rebuilt with only those parts changed — every other entry round-trips
//! byte-for-byte.
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

#[cfg(feature = "docx")]
pub use self::docx_handler::format as docx_format;
#[cfg(feature = "docx")]
pub(crate) use self::docx_loader::DocxLoader;
