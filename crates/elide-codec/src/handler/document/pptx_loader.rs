//! PPTX loader: extract the slide text blocks via [`elide_office`] and hand them
//! to the shared [`ExtractHandler`], retaining the archive for re-pack.

use std::collections::HashMap;
use std::fmt::Write;

use elide_core::modality::text::Text;
use elide_core::{Error, ErrorKind, Result};

use super::pptx_handler::{FORMAT_ID, PptxAddress, PptxEncoder, PptxHandler, pptx_error};
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::{ExtractHandler, ExtractedItem};

/// Loader for PPTX files. Produces one [`PptxHandler`] per input.
#[derive(Debug)]
pub(crate) struct PptxLoader;

#[async_trait::async_trait]
impl Loader<Text> for PptxLoader {
    type Handler = PptxHandler;

    async fn decode(&self, content: ContentData) -> Result<PptxHandler> {
        let archive = content.to_bytes();
        let extraction = elide_office::pptx::Pptx::open(&archive)
            .map_err(pptx_error)?
            .extract();

        // Fail closed on a partial extraction: a non-empty `issues` list means
        // some text-bearing part was not read into blocks, so its text would
        // ship un-redacted. A redaction tool must never do that silently.
        if !extraction.issues.is_empty() {
            let mut parts = String::new();
            for issue in &extraction.issues {
                if !parts.is_empty() {
                    parts.push_str(", ");
                }
                let _ = write!(parts, "{} ({:?})", issue.part, issue.kind);
            }
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("pptx: text-bearing part(s) could not be extracted: {parts}"),
            ));
        }

        let items: Vec<ExtractedItem<PptxAddress>> = extraction
            .blocks
            .into_iter()
            .map(|block| ExtractedItem {
                value: block.text.to_string(),
                address: PptxAddress {
                    part: block.part,
                    span: block.start..block.end,
                    offsets: block.offsets,
                },
                hints: Vec::new(),
            })
            .collect();
        Ok(ExtractHandler::new(
            FORMAT_ID.clone(),
            PptxEncoder {
                archive,
                embeddings: extraction.embeddings,
                replacements: HashMap::new(),
            },
            items,
        ))
    }
}
