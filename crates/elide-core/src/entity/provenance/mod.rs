//! Audit records: the per-entity [`Provenance`] trail.
//!
//! [`Provenance`]: crate::entity::provenance::Provenance

mod attribution;
mod event;
mod rule_match;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::attribution::Attribution;
pub use self::event::{Event, EventKind, ModelEvent, PatternEvent};
pub use self::rule_match::RuleMatch;
use crate::modality::Modality;
use crate::primitive::Confidence;

/// Full audit trail of an [`Entity`]: every [`Event`] in its life, in
/// order.
///
/// This is the model's answer to "full provenance": where Presidio keeps
/// a shallow, optional, per-stage explanation that is stripped by
/// default, a `Provenance` is always present and records the entity's
/// *entire* life as an ordered list of events: each recognizer that
/// found it, the deduplication that fused them, any confidence
/// calibration, and the redaction that hid it. Nothing is collapsed:
/// every recognizer keeps its own recognition event with its location
/// and score.
///
/// The events form a confidence chain (each event's [`after`] is the
/// next's [`before`]) so the final confidence and its full history are
/// always recoverable.
///
/// [`Entity`]: crate::entity::Entity
/// [`after`]: Event::after
/// [`before`]: Event::before
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                   M::Data: Serialize + for<'a> Deserialize<'a>")
)]
pub struct Provenance<M: Modality> {
    /// Events, in the order they happened.
    pub events: Vec<Event<M>>,
}

impl<M: Modality> Provenance<M> {
    /// Provenance seeded with a single (birth) event.
    pub fn new(event: Event<M>) -> Self {
        Self {
            events: vec![event],
        }
    }

    /// Append an event to the trail.
    pub fn record(&mut self, event: Event<M>) {
        self.events.push(event);
    }

    /// Recognition events (pattern / model) that found this entity.
    pub fn recognizers(&self) -> impl Iterator<Item = &Event<M>> {
        self.events.iter().filter(|e| e.is_recognition())
    }

    /// Confidence at the very first event, before any adjustment.
    pub fn original_confidence(&self) -> Option<Confidence> {
        self.events.first().map(|e| e.after)
    }

    /// Confidence after the most recent event: the entity's effective
    /// confidence.
    pub fn final_confidence(&self) -> Option<Confidence> {
        self.events.last().map(|e| e.after)
    }
}

impl<M: Modality> Default for Provenance<M> {
    fn default() -> Self {
        Self { events: Vec::new() }
    }
}
