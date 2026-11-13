//! [`OcrResponse`]: what an OCR backend returns.

use elide_core::backend::Response;
use elide_core::entity::audit::ModelId;

use crate::modality::LayoutRegion;

/// The text an OCR backend recognized in one image, its located regions, and
/// the model that recognized them.
///
/// The [`LayoutRegion`]s come in reading order. They are the core OCR type,
/// so an enricher folds them into a [`Layout`] and onto the call's artifacts
/// without any remapping.
///
/// [`Layout`]: crate::modality::Layout
#[derive(Debug, Clone, Response)]
pub struct OcrResponse {
    /// The model that recognized the regions.
    #[backend(model)]
    pub model: ModelId,
    /// Regions recognized for the request, in reading order.
    pub regions: Vec<LayoutRegion>,
}

impl OcrResponse {
    /// The `regions` `model` recognized.
    #[must_use]
    pub fn new(model: ModelId, regions: Vec<LayoutRegion>) -> Self {
        Self { model, regions }
    }
}
