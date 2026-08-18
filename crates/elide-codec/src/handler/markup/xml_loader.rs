//! XML loader: wires decoded content through the shared [`markup_parser`] into
//! an [`ExtractHandler`] over the [`XmlEncoder`].
//!
//! [`markup_parser`]: super::markup_parser
//! [`ExtractHandler`]: crate::handler::extract::ExtractHandler

use elide_core::Result;
use elide_core::modality::text::Text;

use super::config::MarkupConfig;
use super::markup_parser::build_items;
use super::xml_handler::{FORMAT_ID, XmlEncoder, XmlHandler};
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::ExtractHandler;

/// Loader for XML files. Produces one [`XmlHandler`] per input.
#[derive(Debug)]
pub(crate) struct XmlLoader;

#[async_trait::async_trait]
impl Loader<Text> for XmlLoader {
    type Handler = XmlHandler;

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
