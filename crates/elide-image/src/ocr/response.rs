//! [`OcrResponse`]: what an OCR backend returns.

use elide_core::backend::BackendResponse;

use crate::modality::LayoutRegion;

/// The text an OCR backend recognized in one image: its located regions.
///
/// Wraps the [`LayoutRegion`]s the backend recognized in reading order. These
/// are the core OCR type, so an enricher folds them into a [`Layout`] and onto
/// the call's artifacts without any remapping.
///
/// [`Layout`]: crate::modality::Layout
#[derive(Debug, Clone, Default)]
pub struct OcrResponse {
    /// Regions recognized for the request, in reading order.
    pub regions: Vec<LayoutRegion>,
}

impl OcrResponse {
    /// Construct a response from regions.
    #[must_use]
    pub fn new(regions: Vec<LayoutRegion>) -> Self {
        Self { regions }
    }
}

impl BackendResponse for OcrResponse {}
