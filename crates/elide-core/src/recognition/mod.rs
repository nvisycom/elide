//! Detection: the recognizer contract, its inputs, and the entities it emits.
//!
//! A [`Recognizer`] inspects content and emits entities, each carrying a
//! recognition [`AuditEvent`] in its provenance (its location, confidence,
//! and pattern/model detail). When several recognizers find the same
//! thing, a fusion step (in `elide-detection`) combines their entities into
//! one, concatenating their events and appending a fusion event.
//!
//! [`AuditEvent`]: crate::entity::audit::AuditEvent

mod context;
mod label;
mod languages;
mod scope;
mod subject;

use std::sync::Arc;

use futures::future;

pub use self::context::Context;
pub use self::label::LabelMap;
pub use self::languages::Languages;
pub use self::scope::{Scope, ScopeMetadata};
pub use self::subject::Subject;
use crate::Component;
use crate::entity::Entity;
use crate::error::Result;
use crate::modality::Modality;

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
/// fusion step in `elide-detection`; pruning and orchestration belong to a
/// higher layer, not to the recognizer itself.
///
/// Per call, a recognizer receives the [`Subject`] (the chunk's payload plus
/// its enrichment, detected languages, and hints) and a
/// [`Context`] (the analysis-wide languages, jurisdictions, and labels),
/// and returns the entities it found.
///
/// [`Entity`]: crate::entity::Entity
/// [`AuditEvent`]: crate::entity::audit::AuditEvent
#[async_trait::async_trait]
pub trait Recognizer<M>: Component + Send + Sync
where
    M: Modality,
{
    /// Inspect the [`Subject`] in the given context and return the recognized
    /// entities, in modality-local coordinates.
    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_>) -> Result<Vec<Entity<M>>>;

    /// Recognize over a batch of `subjects` under one shared context, returning
    /// one entity list per subject in the same order.
    ///
    /// The batch is a slice of what [`recognize`](Self::recognize) takes, so a
    /// caller batches subjects it only borrows: a decorator can send a subset, or
    /// mix subjects with ones it builds, without copying any.
    ///
    /// The default runs [`recognize`](Self::recognize) for every subject
    /// concurrently, so a recognizer that calls a provider per subject waits on
    /// the slowest call, not on the sum of them. A recognizer backed by a
    /// provider that accepts several inputs in one
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
        subjects: &[&Subject<M>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        future::try_join_all(subjects.iter().map(|subject| self.recognize(subject, ctx))).await
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
    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_>) -> Result<Vec<Entity<M>>> {
        (**self).recognize(subject, ctx).await
    }

    async fn recognize_batch(
        &self,
        subjects: &[&Subject<M>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        (**self).recognize_batch(subjects, ctx).await
    }
}

/// A shared recognizer is a recognizer.
///
/// Both methods take `&self`, so an [`Arc`] forwards them
/// without interior mutability. This is what lets a caller build an
/// expensive recognizer once and attach the same instance to
/// several analyzers: the built-in pattern set compiles a large
/// regex set, and a deployment running four modalities would
/// otherwise pay for it four times per request.
#[async_trait::async_trait]
impl<M, R> Recognizer<M> for Arc<R>
where
    M: Modality,
    R: Recognizer<M> + ?Sized,
{
    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_>) -> Result<Vec<Entity<M>>> {
        (**self).recognize(subject, ctx).await
    }

    async fn recognize_batch(
        &self,
        subjects: &[&Subject<M>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        (**self).recognize_batch(subjects, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::modality::text::{Text, TextData};

    /// A recognizer that records the most `recognize` calls in flight at once.
    #[derive(Default, Component)]
    #[component(name = "in-flight", version = "0")]
    struct InFlight {
        now: AtomicUsize,
        most: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Recognizer<Text> for InFlight {
        async fn recognize(
            &self,
            _subject: &Subject<Text>,
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.now.fetch_sub(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    /// The default batch runs every subject's `recognize` at once, not one
    /// after another.
    #[tokio::test]
    async fn the_default_batch_recognizes_concurrently() {
        let recognizer = InFlight::default();
        let subjects: Vec<Subject<Text>> = ["a", "b", "c"]
            .into_iter()
            .map(|text| Subject::new(TextData::new(text)))
            .collect();
        let scope = Scope::new();
        let found = recognizer
            .recognize_batch(&subjects.iter().collect::<Vec<_>>(), &Context::new(&scope))
            .await
            .unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(recognizer.most.load(Ordering::SeqCst), 3);
    }
}
