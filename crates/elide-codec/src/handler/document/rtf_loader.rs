//! RTF loader: accepts the bytes and produces the stub handler.
//!
//! No parsing happens yet; see [`RtfHandler`] for the stub's behavior.

use elide_core::Result;
use elide_core::modality::text::Text;

use super::rtf_handler::RtfHandler;
use crate::Loader;
use crate::content::ContentData;

/// Loader producing the stub [`RtfHandler`]. Validation will arrive with
/// the real control-word tokenizer.
#[derive(Debug)]
pub(crate) struct RtfLoader;

#[async_trait::async_trait]
impl Loader<Text> for RtfLoader {
    type Handler = RtfHandler;

    async fn decode(&self, _content: ContentData) -> Result<RtfHandler> {
        Ok(RtfHandler::new())
    }
}
