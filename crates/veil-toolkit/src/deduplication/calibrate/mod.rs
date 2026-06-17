//! Calibration: scale entity confidences by per-recognizer multipliers
//! before fusion.

mod map;

use veil_core::entity::Entity;
use veil_core::modality::Modality;
use veil_core::primitive::Confidence;
use veil_core::provenance::Event;

pub use self::map::CalibrationMap;
use super::{Layer, LayerOutput};

/// The calibration stage: scale each entity's confidence by the
/// multiplier for its *originating* recognizer.
///
/// The originating recognizer is the first detection in the entity's
/// provenance — the one that produced the entity. Calibration is a
/// per-detector statement about score shape, so it keys on whoever
/// detected the entity, not on any later contributor. Results clamp to
/// `[0, 1]`. Drops nothing.
#[derive(Debug, Clone, Default)]
pub struct CalibrateLayer {
    calibration: CalibrationMap,
}

impl CalibrateLayer {
    /// A calibration layer from a map.
    pub fn new(calibration: CalibrationMap) -> Self {
        Self { calibration }
    }
}

impl<M: Modality> Layer<M> for CalibrateLayer {
    fn apply(&self, mut entities: Vec<Entity<M>>) -> LayerOutput<M> {
        if self.calibration.is_empty() {
            return LayerOutput::kept(entities);
        }

        for entity in &mut entities {
            // The originating recognizer is the source of the first
            // recognition event in the entity's provenance.
            let multiplier = entity
                .provenance
                .recognizers()
                .next()
                .and_then(|e| self.calibration.get(e.source.as_str()));

            if let Some(m) = multiplier {
                let before = entity.confidence;
                let after = Confidence::clamped((before.get() as f64 * m) as f32);
                entity.confidence = after;
                entity
                    .provenance
                    .record(Event::calibration(before, after, m));
            }
        }

        LayerOutput::kept(entities)
    }
}
