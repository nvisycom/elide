//! HTML codec side: the [`Format`] descriptor for HTML.
//!
//! HTML runs on the shared markup engine, the same byte-span tokenize-and-splice
//! [`ExtractStream`](elide_codec::extract::ExtractStream) /
//! [`MarkupRecombine`](super::xml_handler::MarkupRecombine), configured leniently
//! (see [`MarkupConfig::lenient`](super::config::MarkupConfig::lenient)). There is
//! no separate HTML stream or recombine type; this module supplies only the
//! [`Format`].

use elide_codec::{Format, FormatId};

use super::HtmlLoader;

/// Stable [`FormatId`] for the HTML codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.html");

/// [`Format`] descriptor registered into `FormatRegistry`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID.clone(), HtmlLoader)
        .with_extensions(["html", "htm"])
        .with_content_types(["text/html"])
}
