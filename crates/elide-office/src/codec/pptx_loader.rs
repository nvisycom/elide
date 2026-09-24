//! PPTX loader: decode via the shared [`ooxml`](super::ooxml) codec adapter.

use elide_codec::Loader;
use elide_codec::content::ContentData;
use elide_core::Result;
use elide_core::modality::text::Text;

use super::ooxml::decode_extract;
use super::pptx_handler::{PptxCodec, PptxHandler};

/// Loader for PPTX files. Produces one [`PptxHandler`] per input.
#[derive(Debug)]
pub(crate) struct PptxLoader;

#[async_trait::async_trait]
impl Loader for PptxLoader {
    type Handler = PptxHandler;
    type Modality = Text;

    async fn decode(&self, content: ContentData) -> Result<PptxHandler> {
        decode_extract::<PptxCodec>(content)
    }
}
