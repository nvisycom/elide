//! The shared markup engine HTML and XML both run on, over `elide-codec`'s
//! extract engine.
//!
//! The [`parser`] reads a document into items (text runs, attribute values,
//! comments) grouped into passages, the [`address`] of each maps its decoded
//! value back onto the raw bytes, and the [`document`] writes back only
//! what a redaction changed. [`config`] says how a dialect is read, [`source`]
//! holds the document and the growing item stream, [`tag`] reads an HTML tag
//! the way a browser does, and [`meta`] says what an HTML `<meta>` tag is.

pub(crate) mod address;
pub(crate) mod config;
pub(crate) mod document;
pub(crate) mod meta;
pub(crate) mod parser;
pub(crate) mod source;
pub(crate) mod tag;
