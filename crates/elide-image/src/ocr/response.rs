//! [`OcrResponse`]: what an OCR backend returns.

use elide_core::backend::{BackendResponse, Meter, Units};

use crate::modality::LayoutRegion;

/// One per-call OCR response from an OCR backend.
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

impl BackendResponse for OcrResponse {
    /// OCR bills per image: one request carries one image, so one call is one
    /// image processed.
    fn units(&self) -> Units {
        Units::from(Meter::Images(1))
    }

    fn output_count(&self) -> Option<u64> {
        Some(self.regions.len() as u64)
    }
}
