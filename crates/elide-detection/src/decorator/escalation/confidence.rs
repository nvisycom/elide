//! [`ConfidenceDecider`]: escalate when the baseline's best finding is weak.

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::ConfidenceThreshold;
use elide_core::recognition::Subject;

use super::Decider;

/// A [`Decider`] that escalates when the baseline's strongest entity falls below
/// a confidence threshold — including when it found nothing at all.
///
/// The reasoning: a confident cheap detection does not need the escalated
/// recognizer, but a sparse or low-confidence one (or no detection) is exactly
/// where the costlier model earns its keep. A model-free policy, so the
/// escalation seam can be exercised with no extra backend.
#[derive(Debug, Clone, Copy)]
pub struct ConfidenceDecider {
    /// Escalate unless some baseline entity's confidence meets this.
    threshold: ConfidenceThreshold,
}

impl ConfidenceDecider {
    /// Escalate unless the baseline produced an entity at or above `threshold`.
    #[must_use]
    pub fn new(threshold: ConfidenceThreshold) -> Self {
        Self { threshold }
    }
}

impl Default for ConfidenceDecider {
    /// Escalates unless the baseline cleared [`ConfidenceThreshold::BASELINE`].
    fn default() -> Self {
        Self::new(ConfidenceThreshold::BASELINE)
    }
}

impl<M: Modality> Decider<M> for ConfidenceDecider {
    fn escalate(&self, _subject: &Subject<M>, baseline: &[Entity<M>]) -> bool {
        // Escalate unless some baseline entity already clears the bar. Empty
        // baseline escalates (no entity clears it), so a chunk the cheap pass
        // found nothing in still gets the escalated look.
        !baseline
            .iter()
            .any(|entity| self.threshold.passes(entity.confidence))
    }
}
