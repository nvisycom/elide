//! [`EscalatingRecognizer`]: run a cheap recognizer always, a costly one only
//! when it is worth it.
//!
//! A [`Recognizer`] that wraps two: a `baseline` that runs on every subject, and
//! an `escalated` one (a hosted LLM/VLM, say) that runs only when a [`Decider`]
//! judges — from the subject and what the baseline found — that the extra cost is
//! warranted. The two results are concatenated; the downstream reconcile layer
//! fuses any overlap, so the decorator owns no dedup policy of its own.
//!
//! The baseline **always** runs, so on the success path a miscalibrated decider
//! costs at worst the escalated recognizer's *additional* findings, never the
//! baseline detection. An error from either recognizer propagates and fails the
//! whole call — no partial result is returned. That fail-safe is the point:
//! escalation adds coverage, it does not gate it.

mod confidence;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};

pub use self::confidence::ConfidenceDecider;
use crate::batch::expect_one_per_input;

/// Decides whether a subject warrants the escalated recognizer, given what the
/// cheap baseline already found.
///
/// Called once per subject, after the baseline has run. Returning `false` keeps
/// only the baseline's entities; `true` also runs the escalated recognizer.
pub trait Decider<M: Modality>: Send + Sync {
    /// Whether this subject warrants the escalated recognizer.
    ///
    /// `baseline` is what the cheap recognizer found for `subject` — typically the
    /// signal a decider reasons over (e.g. escalate when it is sparse or weak).
    fn escalate(&self, subject: &Subject<M>, baseline: &[Entity<M>]) -> bool;
}

/// A [`Recognizer`] that runs a cheap `baseline` always and a costly `escalated`
/// one only when a [`Decider`] says the subject warrants it.
///
/// Both wrapped recognizers recognize the same modality `M`. When recognition
/// succeeds, the baseline's entities are always returned; when the decider
/// escalates, the escalated recognizer's entities are appended. An error from
/// either recognizer propagates without returning partial results. Overlap
/// between the two is left to the reconcile layer, matching the pipeline's "keep
/// the redundant set, reconcile later" model.
pub struct EscalatingRecognizer<B, E, D> {
    baseline: B,
    escalated: E,
    decider: D,
}

impl<B, E, D> EscalatingRecognizer<B, E, D> {
    /// Wrap `baseline` (always run) and `escalated` (run on escalation), with
    /// `decider` choosing per subject.
    #[must_use]
    pub fn new(baseline: B, escalated: E, decider: D) -> Self {
        Self {
            baseline,
            escalated,
            decider,
        }
    }
}

#[async_trait::async_trait]
impl<M, B, E, D> Recognizer<M> for EscalatingRecognizer<B, E, D>
where
    M: Modality,
    B: Recognizer<M>,
    E: Recognizer<M>,
    D: Decider<M>,
{
    /// The baseline's id. Each emitted entity already carries its own
    /// originating recognizer in its audit trail (the baseline's or the
    /// escalated one's), so this is only the decorator's own baseline identity.
    fn id(&self) -> ComponentId {
        self.baseline.id()
    }

    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_>) -> Result<Vec<Entity<M>>> {
        let mut entities = self.baseline.recognize(subject, ctx).await?;
        if self.decider.escalate(subject, &entities) {
            entities.extend(self.escalated.recognize(subject, ctx).await?);
        }
        Ok(entities)
    }

    async fn recognize_batch(
        &self,
        subjects: &[&Subject<M>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        // The baseline batches across all subjects at once; the decider needs
        // its findings, so it runs after them. The subjects it escalates go to
        // the escalated recognizer as one batch.
        let per_subject = self.baseline.recognize_batch(subjects, ctx).await?;
        let mut per_subject =
            expect_one_per_input(self.baseline.id(), per_subject, subjects.len())?;

        let escalated: Vec<usize> = (0..subjects.len())
            .filter(|&i| self.decider.escalate(subjects[i], &per_subject[i]))
            .collect();
        if escalated.is_empty() {
            return Ok(per_subject);
        }
        let batch: Vec<&Subject<M>> = escalated.iter().map(|&i| subjects[i]).collect();
        let found = self.escalated.recognize_batch(&batch, ctx).await?;
        let found = expect_one_per_input(self.escalated.id(), found, batch.len())?;
        for (&i, extra) in escalated.iter().zip(found) {
            per_subject[i].extend(extra);
        }
        Ok(per_subject)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use elide_core::entity::audit::{AuditEvent, PatternRule};
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
        let location = TextLocation::inline(0..1);
        let confidence = Confidence::new(confidence).unwrap();
        let event = AuditEvent::pattern(
            "baseline",
            confidence,
            location.clone(),
            PatternRule {
                name: label.as_str().into(),
                ..PatternRule::default()
            },
        );
        Entity::new(label, location).with_event(event)
    }

    /// A recognizer that replays canned entities and counts how often it ran, so
    /// a test can assert whether escalation reached the escalated recognizer.
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
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.entities.clone())
        }
    }

    fn subject(text: &str) -> Subject<Text> {
        Subject::new(TextData::new(text.to_owned()))
    }

    fn ctx(scope: &Scope) -> Context<'_> {
        Context::new(scope)
    }

    /// A confident baseline detection does not escalate: the escalated
    /// recognizer never runs, and only the baseline's entities are returned.
    #[tokio::test]
    async fn a_confident_baseline_does_not_escalate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let baseline = MockRecognizer::new(vec![entity("EMAIL", 0.9)]);
        let escalated = Spy {
            entities: vec![entity("PERSON", 0.9)],
            calls: calls.clone(),
        };
        let recognizer =
            EscalatingRecognizer::new(baseline, escalated, ConfidenceDecider::default());

        let scope = Scope::new();
        let out = recognizer
            .recognize(&subject("a"), &ctx(&scope))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::Relaxed), 0, "escalated must not run");
        assert_eq!(out.len(), 1, "only the baseline's entity");
    }

    /// A weak baseline detection escalates: the escalated recognizer runs and its
    /// entities are appended to the baseline's.
    #[tokio::test]
    async fn a_weak_baseline_escalates_and_merges() {
        let calls = Arc::new(AtomicUsize::new(0));
        let baseline = MockRecognizer::new(vec![entity("EMAIL", 0.2)]);
        let escalated = Spy {
            entities: vec![entity("PERSON", 0.9)],
            calls: calls.clone(),
        };
        let recognizer =
            EscalatingRecognizer::new(baseline, escalated, ConfidenceDecider::default());

        let scope = Scope::new();
        let out = recognizer
            .recognize(&subject("a"), &ctx(&scope))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::Relaxed), 1, "escalated runs once");
        assert_eq!(out.len(), 2, "baseline + escalated, concatenated");
    }

    /// An empty baseline escalates: a chunk the cheap pass found nothing in still
    /// gets the escalated look.
    #[tokio::test]
    async fn an_empty_baseline_escalates() {
        let calls = Arc::new(AtomicUsize::new(0));
        let baseline = MockRecognizer::new(vec![]);
        let escalated = Spy {
            entities: vec![entity("PERSON", 0.9)],
            calls: calls.clone(),
        };
        let recognizer =
            EscalatingRecognizer::new(baseline, escalated, ConfidenceDecider::default());

        let scope = Scope::new();
        let out = recognizer
            .recognize(&subject("a"), &ctx(&scope))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::Relaxed), 1, "escalated runs on empty");
        assert_eq!(out.len(), 1, "the escalated entity");
    }

    /// The batch path escalates per subject: the escalated recognizer runs only
    /// for the subjects whose baseline result is weak, each result scattered back
    /// to its own position.
    #[tokio::test]
    async fn batch_escalates_per_subject() {
        let calls = Arc::new(AtomicUsize::new(0));
        // Threshold MAX: nothing the baseline finds clears it, so every subject
        // escalates. The whole slice goes to the escalated recognizer's
        // `recognize_batch`, whose default loops `recognize` once per subject.
        let baseline = MockRecognizer::new(vec![entity("EMAIL", 0.9)]);
        let escalated = Spy {
            entities: vec![entity("PERSON", 0.9)],
            calls: calls.clone(),
        };
        let recognizer = EscalatingRecognizer::new(
            baseline,
            escalated,
            ConfidenceDecider::new(elide_core::primitive::ConfidenceThreshold::MAX),
        );

        let scope = Scope::new();
        let subjects = [subject("a"), subject("b"), subject("c")];
        let out = recognizer
            .recognize_batch(&subjects.iter().collect::<Vec<_>>(), &ctx(&scope))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::Relaxed), 3, "one escalated call each");
        assert_eq!(out.len(), 3);
        assert!(
            out.iter().all(|e| e.len() == 2),
            "baseline + escalated each"
        );
    }

    /// Only some subjects escalate: the escalated recognizer gets exactly those,
    /// as one batch, each result scattered to its own position, and the
    /// non-escalated subjects keep only their baseline entities.
    #[tokio::test]
    async fn batch_escalates_only_weak_subjects() {
        // A baseline that flags a weak entity only for subject "b", and nothing
        // for "a"/"c". With the default threshold, "a"/"c" (empty) escalate and
        // "b" (weak) escalates too — so vary it: make the baseline strong, so
        // only the empty subjects escalate.
        struct PerSubject;
        #[async_trait::async_trait]
        impl Recognizer<Text> for PerSubject {
            fn id(&self) -> ComponentId {
                ComponentId::new("per-subject", "1")
            }

            async fn recognize(
                &self,
                subject: &Subject<Text>,
                _ctx: &Context<'_>,
            ) -> Result<Vec<Entity<Text>>> {
                // Only "b" gets a confident baseline hit; "a"/"c" get nothing.
                if subject.data().as_str() == "b" {
                    Ok(vec![entity("EMAIL", 0.9)])
                } else {
                    Ok(vec![])
                }
            }
        }

        /// Records the subjects of every batch it is handed.
        struct Batches {
            seen: Arc<Mutex<Vec<Vec<String>>>>,
        }
        #[async_trait::async_trait]
        impl Recognizer<Text> for Batches {
            fn id(&self) -> ComponentId {
                ComponentId::new("batches", "1")
            }

            async fn recognize(
                &self,
                subject: &Subject<Text>,
                ctx: &Context<'_>,
            ) -> Result<Vec<Entity<Text>>> {
                let mut found = self.recognize_batch(&[subject], ctx).await?;
                Ok(found.pop().unwrap_or_default())
            }

            async fn recognize_batch(
                &self,
                subjects: &[&Subject<Text>],
                _ctx: &Context<'_>,
            ) -> Result<Vec<Vec<Entity<Text>>>> {
                self.seen.lock().unwrap().push(
                    subjects
                        .iter()
                        .map(|subject| subject.data().as_str().to_owned())
                        .collect(),
                );
                Ok(subjects
                    .iter()
                    .map(|_| vec![entity("PERSON", 0.9)])
                    .collect())
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let escalated = Batches { seen: seen.clone() };
        let recognizer =
            EscalatingRecognizer::new(PerSubject, escalated, ConfidenceDecider::default());

        let scope = Scope::new();
        let subjects = [subject("a"), subject("b"), subject("c")];
        let out = recognizer
            .recognize_batch(&subjects.iter().collect::<Vec<_>>(), &ctx(&scope))
            .await
            .unwrap();

        // "a" and "c" escalated (empty baseline), "b" did not (confident), and
        // they reached the escalated recognizer as one batch.
        assert_eq!(
            *seen.lock().unwrap(),
            [vec!["a".to_owned(), "c".to_owned()]]
        );
        assert_eq!(out[0].len(), 1, "a: escalated only");
        assert_eq!(out[1].len(), 1, "b: baseline only, no escalation");
        assert_eq!(out[2].len(), 1, "c: escalated only");
    }
}
