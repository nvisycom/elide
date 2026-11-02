//! The shared markup engine HTML and XML both run on, over `elide-codec`'s
//! extract engine.
//!
//! The [`parser`] reads a document into items (text runs, attribute values,
//! comments) grouped into passages, the [`address`] of each maps its decoded
//! value back onto the raw bytes, and the [`recombine`] step writes back only
//! what a redaction changed. [`config`] says how a dialect is read, [`source`]
//! holds the document and the growing item stream, and [`tag`] reads an HTML
//! tag the way a browser does.

pub(crate) mod address;
pub(crate) mod config;
pub(crate) mod parser;
pub(crate) mod recombine;
pub(crate) mod source;
pub(crate) mod tag;
