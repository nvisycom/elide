//! [`Resilient`]: a [`Recognizer`] decorator that turns its inner's errors into
//! empty results instead of failing the whole analysis.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};

/// Wraps a [`Recognizer`] so that an error from it is logged and swallowed,
/// yielding no entities for the affected subjects rather than aborting the run.
///
/// The analyzer joins its recognizers and fails the whole analysis on the first
/// error, which is the safe default: for a redaction tool, a recognizer that
/// errors has *not* vouched that its subjects are clean, and silently dropping it
/// could leave the entities it would have caught un-redacted. Wrap a recognizer
/// in `Resilient` only where that trade is made deliberately — a flaky,
/// best-effort, or optional recognizer whose failure should degrade coverage
/// rather than stop the pipeline. Recognizers left unwrapped stay fail-fast.
///
/// On failure it emits a `warn` log naming the wrapped recognizer (via its
/// [`id`](Recognizer::id)) and the error, so a dropped recognizer is never
/// silent, then returns empty results. A batch failure yields one empty list per
/// subject, preserving the one-list-per-subject contract the analyzer enforces.
/// It adds no entities and alters none, so it is transparent on the success path.
///
/// Because it is itself a `Recognizer` over the same modality, it drops in
/// anywhere the inner recognizer goes and composes with the other decorators
/// (e.g. wrap an [`EscalatingRecognizer`](crate::escalation::EscalatingRecognizer)
/// to make its whole escalation best-effort).
pub struct Resilient<R> {
    inner: R,
}

impl<R> Resilient<R> {
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
impl<M: Modality, R: Recognizer<M>> Recognizer<M> for Resilient<R> {
    fn id(&self) -> ComponentId {
        self.inner.id()
    }

    async fn recognize(
        &self,
        subject: &Subject<M>,
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
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
        subjects: &[Subject<M>],
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        match self.inner.recognize_batch(subjects, ctx).await {
            Ok(per_subject) => Ok(per_subject),
            Err(error) => {
                tracing::warn!(
                    recognizer = %self.inner.id(),
                    %error,
                    subjects = subjects.len(),
                    "recognizer batch failed; continuing with no entities from it",
                );
                // One empty list per subject: the analyzer requires each
                // recognizer to return exactly one list per subject, and a whole
                // batch failing means none of them got any entities.
                Ok(subjects.iter().map(|_| Vec::new()).collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::{Text, TextData};
    use elide_core::recognition::{Scope, Subject};
    use elide_core::{Error, ErrorKind};

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
            _ctx: &Context<'_, Text>,
        ) -> Result<Vec<Entity<Text>>> {
            Err(Error::new(ErrorKind::Recognition, "boom"))
        }
    }

    fn subject(text: &str) -> Subject<Text> {
        Subject::new(TextData::new(text.to_owned()))
    }

    #[tokio::test]
    async fn a_failing_recognize_yields_no_entities() {
        let recognizer = Resilient::new(AlwaysFails);
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
        let recognizer = Resilient::new(AlwaysFails);
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let subjects = [subject("a"), subject("b"), subject("c")];
        let out = recognizer
            .recognize_batch(&subjects, &ctx)
            .await
            .expect("resilient swallows the error");
        // Exactly one (empty) list per subject — the analyzer's contract.
        assert_eq!(out.len(), subjects.len());
        assert!(out.iter().all(Vec::is_empty));
    }
}
