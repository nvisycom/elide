//! [`ResilientRecognizer`]: a [`Recognizer`] decorator that turns its inner's errors into
//! empty results instead of failing the whole analysis.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};
use futures::future;

/// Wraps a [`Recognizer`] so that an error from it is logged and swallowed,
/// yielding no entities for the affected subjects rather than aborting the run.
///
/// The analyzer joins its recognizers and fails the whole analysis on the first
/// error, which is the safe default: for a redaction tool, a recognizer that
/// errors has *not* vouched that its subjects are clean, and silently dropping it
/// could leave the entities it would have caught un-redacted. Wrap a recognizer
/// in `ResilientRecognizer` only where that trade is made deliberately — a flaky,
/// best-effort, or optional recognizer whose failure should degrade coverage
/// rather than stop the pipeline. Recognizers left unwrapped stay fail-fast.
///
/// On failure it emits a `warn` log naming the wrapped recognizer (via its
/// [`id`](Recognizer::id)) and the error, so a dropped recognizer is never
/// silent, then returns empty results. A failed batch is retried subject by
/// subject, so one subject's failure costs only that subject its findings, and
/// the result keeps the one-list-per-subject contract the analyzer enforces.
/// It adds no entities and alters none, so it is transparent on the success path.
///
/// Because it is itself a `Recognizer` over the same modality, it drops in
/// anywhere the inner recognizer goes and composes with the other decorators
/// (e.g. wrap an [`EscalatingRecognizer`](crate::decorator::escalation::EscalatingRecognizer)
/// to make its whole escalation best-effort).
pub struct ResilientRecognizer<R> {
    inner: R,
}

impl<R> ResilientRecognizer<R> {
    /// Make `inner`'s failures non-fatal: logged and turned into empty results.
    #[must_use]
    pub fn new(inner: R) -> Self {
        Self { inner }
    }

    /// The wrapped recognizer.
    pub fn inner(&self) -> &R {
        &self.inner
    }
}

#[async_trait::async_trait]
impl<M: Modality, R: Recognizer<M>> Recognizer<M> for ResilientRecognizer<R> {
    fn id(&self) -> ComponentId {
        self.inner.id()
    }

    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_>) -> Result<Vec<Entity<M>>> {
        match self.inner.recognize(subject, ctx).await {
            Ok(entities) => Ok(entities),
            Err(error) => {
                tracing::warn!(
                    recognizer = %self.inner.id(),
                    %error,
                    "recognizer failed; continuing with no entities from it",
                );
                Ok(Vec::new())
            }
        }
    }

    async fn recognize_batch(
        &self,
        subjects: &[&Subject<M>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        match self.inner.recognize_batch(subjects, ctx).await {
            Ok(per_subject) => Ok(per_subject),
            Err(error) if subjects.len() > 1 => {
                tracing::warn!(
                    recognizer = %self.inner.id(),
                    %error,
                    subjects = subjects.len(),
                    "recognizer batch failed; retrying each subject on its own",
                );
                // One subject's failure must not cost the others their
                // findings: recognize each on its own (concurrently), so only
                // the subjects that still fail come back empty.
                Ok(
                    future::join_all(subjects.iter().map(|subject| self.recognize(subject, ctx)))
                        .await
                        .into_iter()
                        .map(Result::unwrap_or_default)
                        .collect(),
                )
            }
            Err(error) => {
                tracing::warn!(
                    recognizer = %self.inner.id(),
                    %error,
                    subjects = subjects.len(),
                    "recognizer failed; continuing with no entities from it",
                );
                Ok(subjects.iter().map(|_| Vec::new()).collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::Error;
    use elide_core::entity::LabelRef;
    use elide_core::entity::audit::{AuditEvent, PatternRule};
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::Confidence;
    use elide_core::recognition::{Scope, Subject};

    use super::*;

    /// A recognizer that always errors, to exercise the resilient fallback.
    struct AlwaysFails;

    #[async_trait::async_trait]
    impl Recognizer<Text> for AlwaysFails {
        fn id(&self) -> ComponentId {
            ComponentId::new("always-fails", "0")
        }

        async fn recognize(
            &self,
            _subject: &Subject<Text>,
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            Err(Error::recognition("boom"))
        }
    }

    fn subject(text: &str) -> Subject<Text> {
        Subject::new(TextData::new(text.to_owned()))
    }

    #[tokio::test]
    async fn a_failing_recognize_yields_no_entities() {
        let recognizer = ResilientRecognizer::new(AlwaysFails);
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let out = recognizer
            .recognize(&subject("Jane Doe"), &ctx)
            .await
            .expect("resilient swallows the error");
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn a_failing_batch_yields_one_empty_list_per_subject() {
        let recognizer = ResilientRecognizer::new(AlwaysFails);
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let subjects = [subject("a"), subject("b"), subject("c")];
        let out = recognizer
            .recognize_batch(&subjects.iter().collect::<Vec<_>>(), &ctx)
            .await
            .expect("resilient swallows the error");
        // Exactly one (empty) list per subject — the analyzer's contract.
        assert_eq!(out.len(), subjects.len());
        assert!(out.iter().all(Vec::is_empty));
    }

    /// Fails on the subject `"bad"`, alone or inside a batch, and reports one
    /// finding on any other subject.
    struct FailsOnBad;

    #[async_trait::async_trait]
    impl Recognizer<Text> for FailsOnBad {
        fn id(&self) -> ComponentId {
            ComponentId::new("fails-on-bad", "0")
        }

        async fn recognize(
            &self,
            subject: &Subject<Text>,
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            if subject.data().as_str() == "bad" {
                return Err(Error::recognition("bad subject"));
            }
            let location = TextLocation::inline(0..1);
            let confidence = Confidence::new(0.9).unwrap();
            let event =
                AuditEvent::pattern("t", confidence, location.clone(), PatternRule::default());
            Ok(vec![
                Entity::new(LabelRef::new("THING"), location).with_event(event),
            ])
        }

        async fn recognize_batch(
            &self,
            subjects: &[&Subject<Text>],
            ctx: &Context<'_>,
        ) -> Result<Vec<Vec<Entity<Text>>>> {
            let mut found = Vec::with_capacity(subjects.len());
            for subject in subjects {
                found.push(self.recognize(subject, ctx).await?);
            }
            Ok(found)
        }
    }

    /// One failing subject costs only itself: the batch is retried subject by
    /// subject, and the others keep their findings.
    #[tokio::test]
    async fn a_failing_subject_keeps_the_others_findings() {
        let recognizer = ResilientRecognizer::new(FailsOnBad);
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let subjects = [subject("good"), subject("bad"), subject("also good")];
        let out = recognizer
            .recognize_batch(&subjects.iter().collect::<Vec<_>>(), &ctx)
            .await
            .expect("resilient swallows the error");
        let counts: Vec<usize> = out.iter().map(Vec::len).collect();
        assert_eq!(counts, [1, 0, 1]);
    }

    /// Rejects every batch, even an empty one.
    struct RejectsEveryBatch;

    #[async_trait::async_trait]
    impl Recognizer<Text> for RejectsEveryBatch {
        fn id(&self) -> ComponentId {
            ComponentId::new("rejects-every-batch", "0")
        }

        async fn recognize(
            &self,
            _subject: &Subject<Text>,
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            Err(Error::recognition("rejected"))
        }

        async fn recognize_batch(
            &self,
            _subjects: &[&Subject<Text>],
            _ctx: &Context<'_>,
        ) -> Result<Vec<Vec<Entity<Text>>>> {
            Err(Error::recognition("rejected"))
        }
    }

    /// A failed empty batch still answers one list per subject: none.
    #[tokio::test]
    async fn a_failed_empty_batch_returns_no_lists() {
        let recognizer = ResilientRecognizer::new(RejectsEveryBatch);
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let out = recognizer
            .recognize_batch(&[], &ctx)
            .await
            .expect("resilient swallows the error");
        assert!(out.is_empty());
    }
}
