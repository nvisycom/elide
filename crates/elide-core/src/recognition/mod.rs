//! Detection: recognizers and the entities they emit.
//!
//! A [`Recognizer`] inspects content and emits entities, each carrying a
//! recognition [`Event`] in its provenance (its location, confidence,
//! and pattern/model detail). When several recognizers find the same
//! thing, a fusion step (in `elide`) combines their entities into
//! one, concatenating their events and appending a deduplication event.
//!
//! [`Event`]: crate::entity::provenance::Event

pub mod annotation;
mod artifacts;
mod context;
mod enricher;
mod label;
mod scope;

use std::fmt;
use std::future::Future;

use hipstr::HipStr;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::artifacts::Artifacts;
pub use self::context::RecognizerContext;
pub use self::enricher::Enricher;
pub use self::label::LabelMap;
pub use self::scope::Scope;
use crate::entity::Entity;
use crate::error::Result;
use crate::modality::Modality;

/// Identifies a recognizer (name + version).
///
/// Pairs a stable name with a free-form version string so the audit
/// trail records not just *which* recognizer fired but *which build* of
/// it: a rerun against an updated ruleset or model is then
/// distinguishable from the original. The version is opaque text (a
/// semver, a checkpoint hash, a ruleset date); the core attaches no
/// ordering or comparison semantics to it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct RecognizerId {
    /// Stable, human-readable recognizer name (e.g. `"us-ssn-pattern"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Recognizer's version at the time it ran.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
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

/// Detection layer: inspects content and reports recognized entities.
///
/// Modelled on Presidio's `EntityRecognizer`, generalised to be
/// multimodal (keyed on the [`Modality`] `M`) and provenance-first (the
/// emitted [`Entity`]s carry a recognition [`Event`] in their
/// provenance).
///
/// A recognizer does **not** resolve conflicts or fuse across
/// recognizers; it reports what it sees, in modality-local coordinates.
/// Combining the findings of multiple recognizers is the job of the
/// fusion step in `elide`; pruning and orchestration belong to a
/// higher layer, not to the recognizer itself.
///
/// Per call, a recognizer receives the modality payload (`data`) plus a
/// [`RecognizerContext<M>`] (the call's languages, jurisdictions, label
/// and annotation hints), and returns the entities it found.
///
/// [`Entity`]: crate::entity::Entity
/// [`Event`]: crate::entity::provenance::Event
pub trait Recognizer<M>: Send + Sync
where
    M: Modality,
{
    /// This recognizer's identity (name + version).
    fn id(&self) -> RecognizerId;

    /// Inspect `data` in the given context and return the recognized
    /// entities, in modality-local coordinates.
    fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> impl Future<Output = Result<Vec<Entity<M>>>> + Send;
}
