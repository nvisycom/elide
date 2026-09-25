//! XML loader: wires decoded content through the shared [`markup_parser`] into
//! a leaf [`Document`] whose body is an [`ExtractStream`] recombined by
//! [`MarkupRecombine`].
//!
//! [`markup_parser`]: super::markup_parser
//! [`ExtractStream`]: elide_codec::extract::ExtractStream
//! [`MarkupRecombine`]: super::xml_handler::MarkupRecombine

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader};
use elide_core::Result;

use super::config::MarkupConfig;
use super::markup_parser::build_items;
use super::xml_handler::{FORMAT_ID, markup_document};

/// Loader for XML files. Produces one leaf [`Document`] per input.
#[derive(Debug)]
pub(crate) struct XmlLoader;

#[async_trait::async_trait]
impl DocumentLoader for XmlLoader {
    async fn decode(&self, content: ContentData) -> Result<Document> {
        let text = content.decode()?;
        let items = build_items(&text, MarkupConfig::xml())?;
        Ok(markup_document(FORMAT_ID.clone(), text, items))
    }
}
