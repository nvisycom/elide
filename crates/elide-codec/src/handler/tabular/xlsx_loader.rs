//! XLSX loader: open the workbook via [`elide_office`], extract its cells, and
//! hand them to the [`XlsxHandler`], retaining the archive for re-pack.

use elide_core::Result;
use elide_core::modality::tabular::Tabular;
use elide_office::xlsx::Xlsx;

use super::xlsx_handler::{XlsxCell, XlsxHandler, xlsx_error};
use crate::Loader;
use crate::content::ContentData;

/// Loader for XLSX files. Produces one [`XlsxHandler`] per input.
#[derive(Debug)]
pub(crate) struct XlsxLoader;

#[async_trait::async_trait]
impl Loader<Tabular> for XlsxLoader {
    type Handler = XlsxHandler;

    async fn decode(&self, content: ContentData) -> Result<XlsxHandler> {
        let archive = content.to_bytes();
        let workbook = Xlsx::open(&archive).map_err(xlsx_error)?;
        let cells = workbook
            .extract()
            .map_err(xlsx_error)?
            .into_iter()
            .map(|cell| XlsxCell {
                sheet: cell.sheet.as_str().to_owned(),
                row: cell.row,
                column: cell.column,
                text: cell.text.as_str().to_owned(),
            })
            .collect();
        let text_parts = workbook.text_parts();
        Ok(XlsxHandler::new(archive, cells, text_parts))
    }
}
