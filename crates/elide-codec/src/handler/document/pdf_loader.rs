//! PDF loader: accepts the bytes and produces the stub handler.
//!
//! No parsing happens yet; see [`PdfHandler`] for the stub's behavior.

use elide_core::Result;
use elide_core::modality::text::Text;

use super::pdf_handler::PdfHandler;
use crate::Loader;
use crate::content::ContentData;

/// Loader producing the stub [`PdfHandler`]. Validation will arrive with
/// the real object parser.
#[derive(Debug)]
pub(crate) struct PdfLoader;

impl Loader<Text> for PdfLoader {
    type Handler = PdfHandler;

    async fn decode(&self, _content: ContentData) -> Result<PdfHandler> {
        Ok(PdfHandler::new())
    }
}
