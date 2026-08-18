//! Tree-structured markup formats (HTML, XML) over the shared extract
//! engine.
//!
//! Markup formats differ in their *parser* and *serializer* but share the
//! same redactable units (text nodes, element attributes, comments) and
//! the same streaming/redaction bookkeeping. That bookkeeping is the
//! format-neutral [`extract`] engine ([`ExtractedItem`], [`ExtractHandler`],
//! [`Encoder`]); this module supplies only the HTML and XML parser /
//! serializer pairs on top of it.
//!
//! Each format (e.g. the `html_loader` / `html_handler` pair) supplies a
//! parser that produces the item stream and an [`Encoder`] that splices
//! mutated values back into its native tree; everything between is shared.
//!
//! [`extract`]: crate::handler::extract
//! [`ExtractedItem`]: crate::handler::extract::ExtractedItem
//! [`ExtractHandler`]: crate::handler::extract::ExtractHandler
//! [`Encoder`]: crate::handler::extract::Encoder

// XML is the shared markup engine; `html` enables `xml`, so the engine, config,
// and XML modules compile whenever any markup format is on.
#[cfg(feature = "xml")]
mod config;
#[cfg(feature = "html")]
mod html_handler;
#[cfg(feature = "html")]
mod html_loader;
#[cfg(feature = "xml")]
mod markup_parser;
#[cfg(feature = "xml")]
mod stream;
#[cfg(feature = "xml")]
mod xml_handler;
#[cfg(feature = "xml")]
mod xml_loader;

#[cfg(feature = "html")]
pub use self::html_handler::{
    ScriptPolicy, format as html_format, format_with as html_format_with,
};
#[cfg(feature = "html")]
pub(crate) use self::html_loader::HtmlLoader;
#[cfg(feature = "xml")]
pub use self::xml_handler::format as xml_format;
#[cfg(feature = "xml")]
pub(crate) use self::xml_loader::XmlLoader;
