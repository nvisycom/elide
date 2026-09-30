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

pub use self::context::Context;
pub use self::label::LabelMap;
pub use self::languages::Languages;
pub use self::scope::{Scope, ScopeMetadata};
pub use self::subject::Subject;
use crate::entity::Entity;
use crate::error::Result;
use crate::modality::Modality;
use crate::primitive::ComponentId;

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
/// [`Context<M>`] (the analysis-wide languages, jurisdictions, label
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
    /// entities, in modality-local coordinates.
    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_, M>)
    -> Result<Vec<Entity<M>>>;

    /// Recognize over a batch of `subjects` under one shared context, returning
    /// one entity list per subject in the same order.
    ///
    /// The default recognizes each in turn via [`recognize`](Self::recognize). A
    /// recognizer backed by a provider that accepts several inputs in one
    /// round-trip (a hosted NER or extraction endpoint) overrides this to build
    /// one request per subject and issue a single
    /// [`Backend::call_batch`](crate::backend::Backend::call_batch), turning N
    /// provider round-trips into one. It stays behavior-preserving: each subject's
    /// entities are what a single [`recognize`](Self::recognize) would return, and
    /// a subject the recognizer has nothing to do for (no recognizable text)
    /// yields an empty list.
    ///
    /// # Errors
    ///
    /// The first recognition error; a batched recognizer surfaces a whole-batch
    /// failure the same way.
    async fn recognize_batch(
        &self,
        subjects: &[Subject<M>],
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        let mut per_subject = Vec::with_capacity(subjects.len());
        for subject in subjects {
            per_subject.push(self.recognize(subject, ctx).await?);
        }
        Ok(per_subject)
    }
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
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        (**self).recognize(subject, ctx).await
    }

    async fn recognize_batch(
        &self,
        subjects: &[Subject<M>],
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        (**self).recognize_batch(subjects, ctx).await
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
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        (**self).recognize(subject, ctx).await
    }

    async fn recognize_batch(
        &self,
        subjects: &[Subject<M>],
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        (**self).recognize_batch(subjects, ctx).await
    }
}
