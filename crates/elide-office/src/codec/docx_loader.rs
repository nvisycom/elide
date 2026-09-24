//! DOCX loader: decode via the shared [`ooxml`](super::ooxml) codec adapter.

use elide_codec::Loader;
use elide_codec::content::ContentData;
use elide_core::Result;
use elide_core::modality::text::Text;

use super::docx_handler::{DocxCodec, DocxHandler};
use super::ooxml::decode_extract;

/// Loader for DOCX files. Produces one [`DocxHandler`] per input.
#[derive(Debug)]
pub(crate) struct DocxLoader;

#[async_trait::async_trait]
impl Loader<Text> for DocxLoader {
    type Handler = DocxHandler;

    async fn decode(&self, content: ContentData) -> Result<DocxHandler> {
        decode_extract::<DocxCodec>(content)
    }
}
