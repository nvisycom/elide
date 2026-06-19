//! XLSX loader: accepts the bytes and produces the stub handler.
//!
//! No parsing happens yet; see [`XlsxHandler`] for the stub's behavior.

use elide_core::Result;
use elide_core::modality::tabular::Tabular;

use super::xlsx_handler::XlsxHandler;
use crate::Loader;
use crate::content::ContentData;

/// Loader producing the stub [`XlsxHandler`]. Validation will arrive with
/// the real parser.
#[derive(Debug)]
pub(crate) struct XlsxLoader;

impl Loader<Tabular> for XlsxLoader {
    type Handler = XlsxHandler;

    async fn decode(&self, _content: ContentData) -> Result<XlsxHandler> {
        Ok(XlsxHandler::new())
    }
}
