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
mod languages;
mod scope;
mod subject;

pub use self::context::RecognizerContext;
pub use self::label::LabelMap;
pub use self::languages::Languages;
pub use self::scope::{Scope, ScopeMetadata};
pub use self::subject::Subject;
use crate::entity::Entity;
use crate::error::Result;
use crate::modality::Modality;
use crate::primitive::ComponentId;
#[cfg(feature = "usage")]
use crate::primitive::ModelUsage;

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
/// Per call, a recognizer receives the [`Subject`] (the chunk's payload plus
/// its enrichment, detected languages, and hints) and a
/// [`RecognizerContext<M>`] (the analysis-wide languages, jurisdictions, label
/// and annotation state), and returns the entities it found.
///
/// [`Entity`]: crate::entity::Entity
/// [`AuditEvent`]: crate::entity::audit::AuditEvent
#[async_trait::async_trait]
pub trait Recognizer<M>: Send + Sync
where
    M: Modality,
{
    /// This recognizer's identity (name + version).
    fn id(&self) -> ComponentId;

    /// Inspect the [`Subject`] in the given context and return the recognized
    /// entities, in modality-local coordinates, together with any
    /// model-usage detail the call incurred (see [`Recognition`]).
    async fn recognize(
        &self,
        subject: &Subject<M>,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Recognition<M>>;
}

/// A boxed recognizer is a recognizer, so a caller can hold a
/// `Box<dyn Recognizer<M>>` (e.g. a heterogeneous set of recognizers behind one
/// type) and still call it directly.
#[async_trait::async_trait]
impl<M, R> Recognizer<M> for Box<R>
where
    M: Modality,
    R: Recognizer<M> + ?Sized,
{
    fn id(&self) -> ComponentId {
        (**self).id()
    }

    async fn recognize(
        &self,
        subject: &Subject<M>,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Recognition<M>> {
        (**self).recognize(subject, ctx).await
    }
}

/// A shared recognizer is a recognizer.
///
/// Both methods take `&self`, so an [`Arc`](std::sync::Arc) forwards them
/// without interior mutability. This is what lets a caller build an
/// expensive recognizer once and attach the same instance to
/// several analyzers: the built-in pattern set compiles a large
/// regex set, and a deployment running four modalities would
/// otherwise pay for it four times per request.
#[async_trait::async_trait]
impl<M, R> Recognizer<M> for std::sync::Arc<R>
where
    M: Modality,
    R: Recognizer<M> + ?Sized,
{
    fn id(&self) -> ComponentId {
        (**self).id()
    }

    async fn recognize(
        &self,
        subject: &Subject<M>,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Recognition<M>> {
        (**self).recognize(subject, ctx).await
    }
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
