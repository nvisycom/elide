//! Detection: recognizers and the entities they emit.
//!
//! A [`Recognizer`] inspects content and emits entities, each carrying a
//! recognition [`Event`](crate::provenance::Event) in its provenance
//! (its location, confidence, and pattern/model detail). When several
//! recognizers find the same thing, a fusion step (in `veil-toolkit`)
//! combines their entities into one, concatenating their events and
//! appending a deduplication event.

mod input;
mod label;
mod output;

use std::fmt;
use std::future::Future;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::input::RecognizerInput;
pub use self::label::LabelMap;
pub use self::output::RecognizerOutput;
use crate::error::Error;
use crate::modality::Modality;

/// Identifies a recognizer (name + version).
///
/// Pairs a stable name with a free-form version string so the audit
/// trail records not just *which* recognizer fired but *which build* of
/// it — a rerun against an updated ruleset or model is then
/// distinguishable from the original. The version is opaque text (a
/// semver, a checkpoint hash, a ruleset date) — the core attaches no
/// ordering or comparison semantics to it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RecognizerId {
    /// Stable, human-readable recognizer name (e.g. `"us-ssn-pattern"`).
    pub name: HipStr<'static>,
    /// The recognizer's version at the time it ran.
    pub version: HipStr<'static>,
}

impl RecognizerId {
    /// Construct a recognizer identifier.
    pub fn new(name: impl Into<HipStr<'static>>, version: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

impl fmt::Display for RecognizerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

/// A detection layer: inspects content and reports recognized entities.
///
/// Modelled on Presidio's `EntityRecognizer`, generalised to be
/// multimodal (keyed on the [`Modality`] `M`) and provenance-first (the
/// emitted [`Entity`](crate::entity::Entity)s carry a recognition
/// [`Event`](crate::provenance::Event) in their provenance).
///
/// A recognizer does **not** resolve conflicts or fuse across
/// recognizers — it reports what it sees, in modality-local coordinates.
/// Combining the findings of multiple recognizers is the job of the
/// fusion step in `veil-toolkit`; pruning and orchestration belong to a
/// higher layer, not to the recognizer itself.
///
/// The per-call surface is the [`RecognizerInput<M>`] (the modality
/// payload plus language/jurisdiction/label hints); the result is a
/// [`RecognizerOutput<M>`].
pub trait Recognizer<M>: Send + Sync
where
    M: Modality,
{
    /// This recognizer's identity (name + version).
    fn id(&self) -> RecognizerId;

    /// Inspect the input and return the recognized entities.
    fn recognize(
        &self,
        input: &RecognizerInput<M>,
    ) -> impl Future<Output = Result<RecognizerOutput<M>, Error>> + Send;
}
