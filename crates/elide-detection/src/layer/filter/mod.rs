//! Filtering: dropping entities outside an allow-list of labels or
//! below a confidence threshold.

use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::Modality;
use elide_core::primitive::ConfidenceThreshold;

use super::{Layer, LayerOutput};

/// The filtering stage: drop entities by label allow-list or confidence
/// threshold.
///
/// Both checks are optional and compose with AND — an entity must clear
/// every configured filter to be kept. Unlike reconciliation this is plain
/// configuration, not a strategy, so it is a struct rather than a trait.
#[derive(Debug, Clone, Default)]
pub struct FilterLayer {
    allowed_labels: Option<Vec<LabelRef>>,
    threshold: Option<ConfidenceThreshold>,
}

impl FilterLayer {
    /// A filter that keeps everything (configure with the builders).
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict kept entities to these labels.
    #[must_use]
    pub fn with_allowed_labels(mut self, labels: Vec<LabelRef>) -> Self {
        self.allowed_labels = Some(labels);
        self
    }

    /// Drop entities below this confidence threshold.
    #[must_use]
    pub fn with_threshold(mut self, threshold: ConfidenceThreshold) -> Self {
        self.threshold = Some(threshold);
        self
    }

    /// Whether `entity` clears every configured filter.
    pub fn passes<M: Modality>(&self, entity: &Entity<M>) -> bool {
        if let Some(labels) = &self.allowed_labels
            && !labels.contains(&entity.label)
        {
            return false;
        }
        if let Some(threshold) = self.threshold
            && !threshold.passes(entity.confidence)
        {
            return false;
        }
        true
    }
}

impl<M: Modality> Layer<M> for FilterLayer {
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        let mut kept = Vec::new();
        let mut dropped = Vec::new();
        for entity in entities {
            if self.passes(&entity) {
                kept.push(entity);
            } else {
                dropped.push(entity);
            }
        }
        LayerOutput::split(kept, dropped)
    }
}
