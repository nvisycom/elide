//! PPTX loader: decode via the shared [`ooxml`](super::ooxml) codec adapter.

use elide_core::Result;
use elide_core::modality::text::Text;

use super::ooxml::decode_extract;
use super::pptx_handler::{PptxCodec, PptxHandler};
use crate::Loader;
use crate::content::ContentData;

/// Loader for PPTX files. Produces one [`PptxHandler`] per input.
#[derive(Debug)]
pub(crate) struct PptxLoader;

#[async_trait::async_trait]
impl Loader<Text> for PptxLoader {
    type Handler = PptxHandler;

    async fn decode(&self, content: ContentData) -> Result<PptxHandler> {
        decode_extract::<PptxCodec>(content)
    }
}
