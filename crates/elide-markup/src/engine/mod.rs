//! The shared markup engine HTML and XML both run on, over `elide-codec`'s
//! extract engine.
//!
//! The [`parser`] reads a document into items (text runs, attribute values,
//! comments) grouped into passages, the [`address`] of each maps its decoded
//! value back onto the raw bytes ([`percent`] decodes an attribute value's
//! `%XX` escapes), and the [`document`] writes back only
//! what a redaction changed. [`config`] says how a dialect is read, [`reader`]
//! drives quick-xml for it, [`sink`] holds the growing item stream, [`tag`]
//! reads a start tag (an HTML one the way a browser does), and [`meta`] says
//! what an HTML `<meta>` tag is.

pub(crate) mod address;
pub(crate) mod config;
pub(crate) mod document;
pub(crate) mod meta;
pub(crate) mod parser;
pub(crate) mod percent;
pub(crate) mod reader;
pub(crate) mod sink;
pub(crate) mod tag;
#[cfg(test)]
pub(crate) mod test_util;
