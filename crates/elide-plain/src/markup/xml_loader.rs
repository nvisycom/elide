//! XML loader: wires decoded content through the shared [`markup_parser`] into
//! an [`ExtractHandler`] over the [`XmlEncoder`].
//!
//! [`markup_parser`]: super::markup_parser
//! [`ExtractHandler`]: elide_codec::extract::ExtractHandler

use elide_codec::Loader;
use elide_codec::content::ContentData;
use elide_codec::extract::ExtractHandler;
use elide_core::Result;
use elide_core::modality::text::Text;

use super::config::MarkupConfig;
use super::markup_parser::build_items;
use super::xml_handler::{FORMAT_ID, XmlEncoder, XmlHandler};

/// Loader for XML files. Produces one [`XmlHandler`] per input.
#[derive(Debug)]
pub(crate) struct XmlLoader;

#[async_trait::async_trait]
impl Loader for XmlLoader {
    type Handler = XmlHandler;
    type Modality = Text;

    async fn decode(&self, content: ContentData) -> Result<XmlHandler> {
        let text = content.decode()?;
        let items = build_items(&text, MarkupConfig::xml())?;
        Ok(ExtractHandler::new(
            FORMAT_ID.clone(),
            XmlEncoder { raw: text },
            items,
        ))
    }
}
