//! HTML codec side: the [`Format`] descriptor for HTML.
//!
//! HTML runs on the shared markup engine, the same byte-span tokenize-and-splice
//! [`ExtractStream`](elide_codec::extract::ExtractStream) /
//! [`MarkupDocument`](crate::engine::document::MarkupDocument), configured leniently
//! (see [`MarkupConfig::lenient`](crate::engine::config::MarkupConfig::lenient)). There is
//! no separate HTML stream or document type; this module supplies only the
//! [`Format`].

use elide_codec::{Format, FormatId};

use super::HtmlLoader;

/// Stable [`FormatId`] for the HTML codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.html");

/// The HTML [`Format`], resolved by `.html`/`.htm` and `text/html`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID, HtmlLoader)
        .with_extensions(["html", "htm"])
        .with_content_types(["text/html"])
}
