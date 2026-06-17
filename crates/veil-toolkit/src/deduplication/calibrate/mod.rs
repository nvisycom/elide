//! Calibration: scale entity confidences by per-recognizer multipliers
//! before fusion.

mod map;

pub use self::map::CalibrationMap;

use veil_core::entity::Entity;
use veil_core::modality::Modality;
use veil_core::primitive::Confidence;

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
            let multiplier = entity
                .provenance
                .detections
                .first()
                .and_then(|d| self.calibration.get(d.recognizer.name.as_str()));

            if let Some(m) = multiplier {
                entity.confidence =
                    Confidence::clamped((entity.confidence.get() as f64 * m) as f32);
            }
        }

        LayerOutput::kept(entities)
    }
}
