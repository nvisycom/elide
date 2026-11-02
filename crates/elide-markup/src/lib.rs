#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// The engine and the sanitizer compile with either format; `html` enables
// `xml`. The character reference decoder needs neither, so a format that reads
// XML another way can use it alone.
#[cfg(feature = "xml")]
mod engine;
#[cfg(feature = "html")]
mod html;
mod references;
#[cfg(feature = "xml")]
mod sanitize;
#[cfg(feature = "xml")]
mod xml;

#[cfg(feature = "html")]
#[cfg_attr(docsrs, doc(cfg(feature = "html")))]
pub use self::html::format as html_format;
pub use self::references::{CharacterReferences, decode_references};
#[cfg(feature = "xml")]
#[cfg_attr(docsrs, doc(cfg(feature = "xml")))]
pub use self::xml::format as xml_format;
