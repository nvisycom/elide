//! The XML [`Format`] descriptor, over the shared markup engine.

use elide_codec::{Format, FormatId};

use super::XmlLoader;

/// Stable [`FormatId`] for the XML codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.xml");

/// [`Format`] descriptor registered into `FormatRegistry`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID, XmlLoader)
        .with_extensions(["xml"])
        .with_content_types(["application/xml", "text/xml"])
}
