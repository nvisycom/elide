//! Detection: recognizers and the entities they emit.
//!
//! A [`Recognizer`] inspects content and emits entities, each carrying a
//! recognition [`AuditEvent`] in its provenance (its location, confidence,
//! and pattern/model detail). When several recognizers find the same
//! thing, a fusion step (in `elide`) combines their entities into
//! one, concatenating their events and appending a deduplication event.
//!
//! [`AuditEvent`]: crate::entity::audit::AuditEvent

pub mod annotation;
mod context;
mod label;
mod scope;

use std::fmt;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::context::RecognizerContext;
pub use self::label::LabelMap;
pub use self::scope::{Scope, ScopeMetadata};
use crate::entity::Entity;
use crate::error::Result;
use crate::modality::Modality;
#[cfg(feature = "usage")]
use crate::primitive::ModelUsage;

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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
/// emitted [`Entity`]s carry a recognition [`AuditEvent`] in their
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
/// [`AuditEvent`]: crate::entity::audit::AuditEvent
#[async_trait::async_trait]
pub trait Recognizer<M>: Send + Sync
where
    M: Modality,
{
    /// This recognizer's identity (name + version).
    fn id(&self) -> RecognizerId;

    /// Inspect `data` in the given context and return the recognized
    /// entities, in modality-local coordinates, together with any
    /// model-usage detail the call incurred (see [`Recognition`]).
    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Recognition<M>>;
}

/// What a [`Recognizer`] returns from one call.
///
/// The entities it found. Under the `usage` feature it also carries the
/// `ModelUsage` the call cost, which a model-backed recognizer attaches with
/// `with_model_usage`.
#[derive(Debug, Clone)]
pub struct Recognition<M: Modality> {
    /// The recognized entities, in modality-local coordinates.
    pub entities: Vec<Entity<M>>,
    /// Model / token detail for a model-backed recognizer; `None` otherwise.
    #[cfg(feature = "usage")]
    pub model_usage: Option<ModelUsage>,
}

impl<M: Modality> Recognition<M> {
    /// A recognition carrying `entities` (and, under the `usage` feature, no
    /// model usage yet, attach it with `with_model_usage`).
    pub fn new(entities: Vec<Entity<M>>) -> Self {
        Self {
            entities,
            #[cfg(feature = "usage")]
            model_usage: None,
        }
    }

    /// Attach the [`ModelUsage`] this recognition cost (the model-backed path).
    #[cfg(feature = "usage")]
    #[must_use]
    pub fn with_model_usage(mut self, model_usage: ModelUsage) -> Self {
        self.model_usage = Some(model_usage);
        self
    }
}

impl<M: Modality> From<Vec<Entity<M>>> for Recognition<M> {
    /// Entities with no model usage, the pure-CPU recognizer case.
    fn from(entities: Vec<Entity<M>>) -> Self {
        Self::new(entities)
    }
}

impl<M: Modality> Default for Recognition<M> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}
