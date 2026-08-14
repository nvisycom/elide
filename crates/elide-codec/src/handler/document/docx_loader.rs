//! DOCX loader: extract the body's text blocks via [`elide_docx`] and hand
//! them to the shared [`ExtractHandler`], retaining the archive for re-pack.

use std::collections::HashMap;

use elide_core::Result;
use elide_core::modality::text::Text;

use super::docx_handler::{DocxAddress, DocxEncoder, DocxHandler, FORMAT_ID, docx_error};
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::{ExtractHandler, ExtractedItem};

/// Loader for DOCX files. Produces one [`DocxHandler`] per input.
#[derive(Debug)]
pub(crate) struct DocxLoader;

#[async_trait::async_trait]
impl Loader<Text> for DocxLoader {
    type Handler = DocxHandler;

    async fn decode(&self, content: ContentData) -> Result<DocxHandler> {
        let archive = content.to_bytes();
        let extraction = elide_docx::Docx::open(&archive)
            .map_err(docx_error)?
            .extract();
        let items: Vec<ExtractedItem<DocxAddress>> = extraction
            .blocks
            .into_iter()
            .map(|block| ExtractedItem {
                value: block.text.to_string(),
                address: DocxAddress {
                    part: block.part,
                    span: block.start..block.end,
                },
                hints: Vec::new(),
            })
            .collect();
        Ok(ExtractHandler::new(
            FORMAT_ID.clone(),
            DocxEncoder {
                archive,
                replacements: HashMap::new(),
            },
            items,
        ))
    }
}
