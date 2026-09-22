//! [`OcrResponse`]: what an [`OcrBackend`] returns.
//!
//! [`OcrBackend`]: super::OcrBackend

use crate::modality::LayoutBlock;

/// One per-call OCR response from an [`OcrBackend`].
///
/// Wraps the [`LayoutBlock`]s the backend recognized in reading order. These
/// are the core OCR type, so an enricher folds them into an [`Layout`] and
/// onto the call's artifacts without any remapping.
///
/// [`OcrBackend`]: super::OcrBackend
/// [`Layout`]: crate::modality::Layout
#[derive(Debug, Clone, Default)]
pub struct OcrResponse {
    /// Blocks recognized for the request, in reading order.
    pub blocks: Vec<LayoutBlock>,
}

impl OcrResponse {
    /// Construct a response from blocks.
    #[must_use]
    pub fn new(blocks: Vec<LayoutBlock>) -> Self {
        Self { blocks }
    }
}
