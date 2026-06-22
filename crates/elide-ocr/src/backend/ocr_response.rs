//! [`OcrResponse`]: what an [`OcrBackend`] returns.
//!
//! [`OcrBackend`]: super::OcrBackend

use elide_core::primitive::OcrBlock;

/// One per-call OCR response from an [`OcrBackend`].
///
/// Wraps the [`OcrBlock`]s the backend recognized in reading order. These
/// are the core OCR type, so an enricher folds them into an [`OcrText`] and
/// onto the call's artifacts without any remapping.
///
/// [`OcrBackend`]: super::OcrBackend
/// [`OcrText`]: elide_core::primitive::OcrText
#[derive(Debug, Clone, Default)]
pub struct OcrResponse {
    /// Blocks recognized for the request, in reading order.
    pub blocks: Vec<OcrBlock>,
}

impl OcrResponse {
    /// Construct a response from blocks.
    #[must_use]
    pub fn new(blocks: Vec<OcrBlock>) -> Self {
        Self { blocks }
    }
}
