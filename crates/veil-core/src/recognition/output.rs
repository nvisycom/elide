//! [`RecognizerOutput<M>`]: per-call output of a [`Recognizer`].
//!
//! Wraps the emitted entities in a named struct (rather than a bare
//! `Vec<Entity<M>>`) so future per-call metadata — drop counters,
//! telemetry, partial-failure flags — can land alongside without
//! churning every recognizer signature.
//!
//! [`Recognizer`]: super::Recognizer

use crate::entity::Entity;
use crate::modality::Modality;

/// Per-call output of a [`Recognizer`].
///
/// [`Recognizer`]: super::Recognizer
#[derive(Debug, Clone)]
pub struct RecognizerOutput<M: Modality> {
    /// Entities the recognizer emitted in modality-local coordinates.
    pub entities: Vec<Entity<M>>,
}

impl<M: Modality> RecognizerOutput<M> {
    /// Construct from the underlying entity list.
    #[must_use]
    pub fn new(entities: Vec<Entity<M>>) -> Self {
        Self { entities }
    }

    /// Empty output — no entities emitted.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }
}

impl<M: Modality> Default for RecognizerOutput<M> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<M: Modality> From<Vec<Entity<M>>> for RecognizerOutput<M> {
    fn from(entities: Vec<Entity<M>>) -> Self {
        Self::new(entities)
    }
}

impl<M: Modality> From<RecognizerOutput<M>> for Vec<Entity<M>> {
    fn from(output: RecognizerOutput<M>) -> Self {
        output.entities
    }
}
