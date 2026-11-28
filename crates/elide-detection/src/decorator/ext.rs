//! [`RecognizerExt`]: wrap a recognizer in a decorator by method, in the order
//! the decorators apply.

use elide_core::Component;

use super::calibrated::{Calibrated, Calibration};
use super::chunked::{Chunked, WordWindows};
use super::escalating::Escalating;
use super::resilient::Resilient;

/// Wraps a recognizer in this crate's decorators by method.
///
/// Each method is the decorator's constructor with the recognizer as its first
/// argument, so a stack of decorators reads in the order they wrap, innermost
/// first, instead of inside out, as `BackendExt` in `elide-backend` does for
/// backends:
///
/// ```ignore
/// let recognizer = ner
///     .chunked_with(CharWindows::for_gliner())
///     .calibrated(Calibration::scale(SCALE))
///     .resilient();
/// ```
///
/// Implemented for every [`Component`], the supertrait of every recognizer,
/// rather than for each modality's `Recognizer`: a decorator wraps a
/// recognizer whatever modality it serves, and a recognizer serving several
/// would leave the modality to name at every call.
pub trait RecognizerExt: Component + Sized {
    /// Log this recognizer's errors and carry on without its findings; see
    /// [`Resilient`].
    fn resilient(self) -> Resilient<Self> {
        Resilient::new(self)
    }

    /// Split an over-long text into word windows; see [`Chunked`].
    fn chunked(self) -> Chunked<Self, WordWindows> {
        Chunked::new(self)
    }

    /// Split an over-long text with `chunking`; see [`Chunked`].
    fn chunked_with<C>(self, chunking: C) -> Chunked<Self, C> {
        Chunked::with_chunking(self, chunking)
    }

    /// Scale this recognizer's findings by `calibration`; see
    /// [`Calibrated`].
    fn calibrated(self, calibration: Calibration) -> Calibrated<Self> {
        Calibrated::new(self, calibration)
    }

    /// Run `escalated` as well on the subjects `decider` picks from this
    /// recognizer's findings; see [`Escalating`].
    fn escalating<E, D>(self, escalated: E, decider: D) -> Escalating<Self, E, D> {
        Escalating::new(self, escalated, decider)
    }
}

impl<R: Component> RecognizerExt for R {}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{Pattern, PatternRule};
    use elide_core::entity::{Birth, Entity, builtins};
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::{ComponentId, Confidence};
    use elide_core::recognition::{Context, Recognizer, Scope, Subject};
    use elide_core::{Error, Result};

    use super::*;
    use crate::decorator::calibrated::Multiplier;

    /// Finds one person name at full confidence, or fails, as built.
    #[derive(Component)]
    #[component(name = "one-find", version = "0")]
    struct OneFind {
        fails: bool,
    }

    #[async_trait::async_trait]
    impl Recognizer<Text> for OneFind {
        async fn recognize(
            &self,
            _subject: &Subject<Text>,
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            if self.fails {
                return Err(Error::recognition("boom"));
            }
            let at = TextLocation::inline(0..4);
            Ok(vec![Entity::new(Birth::new(
                Confidence::MAX,
                Pattern::new(
                    ComponentId::new("one-find", "1"),
                    builtins::PERSON_NAME.to_ref(),
                    at,
                    PatternRule::default(),
                ),
            ))])
        }
    }

    /// A chain wraps innermost first: calibrated, then made resilient.
    #[tokio::test]
    async fn a_chain_wraps_in_order() {
        const HALF: Multiplier = Multiplier::new(0.5).unwrap();
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let subject = Subject::<Text>::new(TextData::new("Jane".to_owned()));

        let found = OneFind { fails: false }
            .calibrated(Calibration::scale(HALF))
            .resilient()
            .recognize(&subject, &ctx)
            .await
            .unwrap();
        assert_eq!(found[0].confidence(), Confidence::new(0.5).unwrap());

        let failed = OneFind { fails: true }
            .calibrated(Calibration::scale(HALF))
            .resilient()
            .recognize(&subject, &ctx)
            .await
            .unwrap();
        assert!(failed.is_empty());
    }
}
