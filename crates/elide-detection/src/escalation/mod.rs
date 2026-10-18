//! [`EscalatingRecognizer`]: run a cheap recognizer always, a costly one only
//! when it is worth it.
//!
//! A [`Recognizer`] that wraps two: a `fallback` that runs on every subject, and
//! an `expensive` one (a hosted LLM/VLM, say) that runs only when a [`Decider`]
//! judges — from the subject and what the fallback found — that the extra cost is
//! warranted. The two results are concatenated; the downstream reconcile layer
//! fuses any overlap, so the decorator owns no dedup policy of its own.
//!
//! The fallback **always** runs, so a miscalibrated decider costs at worst the
//! expensive recognizer's *additional* findings — never the baseline detection.
//! That fail-safe is the point: escalation adds coverage, it does not gate it.

mod confidence;

use std::sync::Arc;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};

pub use self::confidence::ConfidenceDecider;

/// Decides whether a subject warrants the expensive recognizer, given what the
/// cheap fallback already found.
///
/// Called once per subject, after the fallback has run. Returning `false` keeps
/// only the fallback's entities; `true` also runs the expensive recognizer.
pub trait Decider<M: Modality>: Send + Sync {
    /// Whether to escalate this subject to the expensive recognizer.
    ///
    /// `fallback` is what the cheap recognizer found for `subject` — typically the
    /// signal a decider reasons over (e.g. escalate when it is sparse or weak).
    fn escalate(&self, subject: &Subject<M>, fallback: &[Entity<M>]) -> bool;
}

/// A [`Recognizer`] that runs a cheap `fallback` always and a costly `expensive`
/// one only when a [`Decider`] says the subject warrants it.
///
/// Both wrapped recognizers recognize the same modality `M`. The fallback's
/// entities are always returned; when the decider escalates, the expensive
/// recognizer's entities are appended. Overlap between the two is left to the
/// reconcile layer, matching the pipeline's "keep the redundant set, reconcile
/// later" model.
pub struct EscalatingRecognizer<M: Modality, D> {
    fallback: Arc<dyn Recognizer<M>>,
    expensive: Arc<dyn Recognizer<M>>,
    decider: D,
}

impl<M: Modality, D: Decider<M>> EscalatingRecognizer<M, D> {
    /// Wrap `fallback` (always run) and `expensive` (run on escalation), with
    /// `decider` choosing per subject.
    #[must_use]
    pub fn new(
        fallback: Arc<dyn Recognizer<M>>,
        expensive: Arc<dyn Recognizer<M>>,
        decider: D,
    ) -> Self {
        Self {
            fallback,
            expensive,
            decider,
        }
    }
}

#[async_trait::async_trait]
impl<M: Modality, D: Decider<M>> Recognizer<M> for EscalatingRecognizer<M, D> {
    /// The fallback's id. Each emitted entity already carries its own
    /// originating recognizer in its audit trail (the fallback's or the
    /// expensive one's), so this is only the decorator's own fallback identity.
    fn id(&self) -> ComponentId {
        self.fallback.id()
    }

    async fn recognize(
        &self,
        subject: &Subject<M>,
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        let mut entities = self.fallback.recognize(subject, ctx).await?;
        if self.decider.escalate(subject, &entities) {
            entities.extend(self.expensive.recognize(subject, ctx).await?);
        }
        Ok(entities)
    }

    async fn recognize_batch(
        &self,
        subjects: &[Subject<M>],
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        // The fallback batches across all subjects at once (preserving a
        // provider-backed fallback's own coalescing); the expensive recognizer
        // then runs per escalated subject. A filtered, non-contiguous subset of
        // `subjects` can't be handed to `recognize_batch` as one slice (`Subject`
        // is deliberately not `Clone`), so escalation calls `recognize` per
        // subject — which is what the expensive recognizer's own default batch
        // does anyway.
        let mut per_subject = self.fallback.recognize_batch(subjects, ctx).await?;
        for (subject, entities) in subjects.iter().zip(per_subject.iter_mut()) {
            if self.decider.escalate(subject, entities) {
                entities.extend(self.expensive.recognize(subject, ctx).await?);
            }
        }
        Ok(per_subject)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::mocks::MockRecognizer;
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::{ComponentId, Confidence};
    use elide_core::recognition::{Context, Scope};

    use super::*;

    /// A text entity labelled `label` with the given confidence, located at
    /// `[0, 1)`.
    fn entity(label: &str, confidence: f32) -> Entity<Text> {
        let label = LabelRef::new(label.to_owned());
        let location = TextLocation::new(0, 1);
        let confidence = Confidence::new(confidence).unwrap();
        let event = AuditEvent::pattern(
            "fallback",
            confidence,
            location.clone(),
            PatternEvent {
                name: label.as_str().into(),
                ..PatternEvent::default()
            },
        );
        Entity::new(label, location, AuditLog::new(event))
    }

    /// A recognizer that replays canned entities and counts how often it ran, so
    /// a test can assert whether escalation reached the expensive recognizer.
    struct Spy {
        entities: Vec<Entity<Text>>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Recognizer<Text> for Spy {
        fn id(&self) -> ComponentId {
            ComponentId::new("spy", "1")
        }

        async fn recognize(
            &self,
            _subject: &Subject<Text>,
            _ctx: &Context<'_, Text>,
        ) -> Result<Vec<Entity<Text>>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.entities.clone())
        }
    }

    fn subject(text: &str) -> Subject<Text> {
        Subject::new(TextData::new(text.to_owned()))
    }

    fn ctx(scope: &Scope) -> Context<'_, Text> {
        Context::new(scope)
    }

    /// A confident fallback detection does not escalate: the expensive
    /// recognizer never runs, and only the fallback's entities are returned.
    #[tokio::test]
    async fn a_confident_fallback_does_not_escalate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fallback = MockRecognizer::new(vec![entity("EMAIL", 0.9)]);
        let expensive = Spy {
            entities: vec![entity("PERSON", 0.9)],
            calls: calls.clone(),
        };
        let recognizer = EscalatingRecognizer::new(
            Arc::new(fallback),
            Arc::new(expensive),
            ConfidenceDecider::default(),
        );

        let scope = Scope::new();
        let out = recognizer
            .recognize(&subject("a"), &ctx(&scope))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::Relaxed), 0, "expensive must not run");
        assert_eq!(out.len(), 1, "only the fallback's entity");
    }

    /// A weak fallback detection escalates: the expensive recognizer runs and its
    /// entities are appended to the fallback's.
    #[tokio::test]
    async fn a_weak_fallback_escalates_and_merges() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fallback = MockRecognizer::new(vec![entity("EMAIL", 0.2)]);
        let expensive = Spy {
            entities: vec![entity("PERSON", 0.9)],
            calls: calls.clone(),
        };
        let recognizer = EscalatingRecognizer::new(
            Arc::new(fallback),
            Arc::new(expensive),
            ConfidenceDecider::default(),
        );

        let scope = Scope::new();
        let out = recognizer
            .recognize(&subject("a"), &ctx(&scope))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::Relaxed), 1, "expensive runs once");
        assert_eq!(out.len(), 2, "fallback + expensive, concatenated");
    }

    /// An empty fallback escalates: a chunk the cheap pass found nothing in still
    /// gets the expensive look.
    #[tokio::test]
    async fn an_empty_fallback_escalates() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fallback = MockRecognizer::new(vec![]);
        let expensive = Spy {
            entities: vec![entity("PERSON", 0.9)],
            calls: calls.clone(),
        };
        let recognizer = EscalatingRecognizer::new(
            Arc::new(fallback),
            Arc::new(expensive),
            ConfidenceDecider::default(),
        );

        let scope = Scope::new();
        let out = recognizer
            .recognize(&subject("a"), &ctx(&scope))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::Relaxed), 1, "expensive runs on empty");
        assert_eq!(out.len(), 1, "the expensive entity");
    }

    /// The batch path escalates per subject: the expensive recognizer runs only
    /// for the subjects whose fallback result is weak, each result scattered back
    /// to its own position.
    #[tokio::test]
    async fn batch_escalates_per_subject() {
        let calls = Arc::new(AtomicUsize::new(0));
        // Fallback is confident for every subject; override escalation by using a
        // threshold that nothing clears, so every subject escalates — then assert
        // one expensive call per subject.
        let fallback = MockRecognizer::new(vec![entity("EMAIL", 0.9)]);
        let expensive = Spy {
            entities: vec![entity("PERSON", 0.9)],
            calls: calls.clone(),
        };
        let recognizer = EscalatingRecognizer::new(
            Arc::new(fallback),
            Arc::new(expensive),
            ConfidenceDecider::new(elide_core::primitive::ConfidenceThreshold::MAX),
        );

        let scope = Scope::new();
        let subjects = [subject("a"), subject("b"), subject("c")];
        let out = recognizer
            .recognize_batch(&subjects, &ctx(&scope))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::Relaxed), 3, "one expensive call each");
        assert_eq!(out.len(), 3);
        assert!(
            out.iter().all(|e| e.len() == 2),
            "fallback + expensive each"
        );
    }
}
