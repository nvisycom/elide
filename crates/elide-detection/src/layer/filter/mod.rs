//! Filtering: dropping entities below a confidence threshold.

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::ConfidenceThreshold;

use super::{Layer, LayerOutput};

/// The filtering stage: drop entities below a confidence threshold.
///
/// Which labels are detected is the request's catalog, which the analyzer
/// already applies to its output; this stage only drops the weak findings.
#[derive(Debug, Clone, Copy)]
pub struct ThresholdLayer {
    threshold: ConfidenceThreshold,
}

impl ThresholdLayer {
    /// A filter dropping entities below `threshold`.
    #[must_use]
    pub fn new(threshold: ConfidenceThreshold) -> Self {
        Self { threshold }
    }

    /// Whether `entity` clears the threshold.
    pub fn passes<M: Modality>(&self, entity: &Entity<M>) -> bool {
        self.threshold.passes(entity.confidence)
    }
}

impl Default for ThresholdLayer {
    /// A filter at [`ConfidenceThreshold::BASELINE`].
    fn default() -> Self {
        Self::new(ConfidenceThreshold::BASELINE)
    }
}

impl<M: Modality> Layer<M> for ThresholdLayer {
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        let (kept, dropped) = entities.into_iter().partition(|entity| self.passes(entity));
        LayerOutput::split(kept, dropped)
    }
}
