//! [`ContextEnhanced`]: post-recognition keyword-boost wrapper for any
//! text [`Recognizer`].
//!
//! Composes an inner recognizer with an [`Enhancer`]: the wrapper
//! delegates `recognize` to the inner, then runs the enhancer over the
//! produced entities. Equivalent to "the recognizer owns its boosting"
//! without each recognizer reimplementing the enhancement step.
//!
//! Typical use:
//!
//! ```ignore
//! let inner = MyRecognizer::new(...);
//! let enhancer = Enhancer::new(rules, SubstringMatcher);
//! let recognizer = ContextEnhanced::new(inner, enhancer);
//! ```
//!
//! The wrapper implements [`Recognizer`]`<Text>` so the engine never has
//! to know boosting happened.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::TextBacked;
use elide_core::modality::text::TextData;
use elide_core::recognition::{Recognizer, RecognizerContext, RecognizerId};

use super::Tokens;
use crate::{Context, Enhancer};

/// Wraps a text [`Recognizer`] with a post-recognition [`Enhancer`] pass.
///
/// Implements [`Recognizer`]`<Text>` so the wrapped recognizer is a
/// drop-in replacement.
///
/// Assumes the inner recognizer emits entities whose byte offsets index
/// into `data.text` (the standard text-recognizer contract).
/// The wrapper reads the same `&str` for the keyword-window walk; a
/// recognizer that emitted entities relative to a different coordinate
/// space would surface stale or panic-on-slice offsets.
pub struct ContextEnhanced<R> {
    inner: R,
    enhancer: Enhancer,
}

impl<R> ContextEnhanced<R> {
    /// Wrap `inner` with `enhancer`. After `recognize` produces
    /// entities, `enhancer` runs over them in place.
    pub fn new(inner: R, enhancer: Enhancer) -> Self {
        Self { inner, enhancer }
    }

    /// Borrow the wrapped recognizer.
    pub fn inner(&self) -> &R {
        &self.inner
    }

    /// Borrow the enhancer applied to the inner recognizer's output.
    pub fn enhancer(&self) -> &Enhancer {
        &self.enhancer
    }
}

impl<M: TextBacked, R> Recognizer<M> for ContextEnhanced<R>
where
    R: Recognizer<M> + 'static,
{
    fn id(&self) -> RecognizerId {
        self.inner.id()
    }

    async fn recognize(
        &self,
        data: &TextData,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        let mut entities = self.inner.recognize(data, ctx).await?;
        if self.enhancer.is_empty() {
            return Ok(entities);
        }
        let mut context = Context::new(data.text.as_str()).with_hints(&ctx.context_hints);
        if let Some(tokens) = ctx.artifacts.get::<Tokens>() {
            context = context.with_tokens(tokens.as_slice());
        }
        if let Some(language) = ctx.primary_language() {
            context = context.with_language(language);
        }
        self.enhancer.enhance(&mut entities, &context);
        Ok(entities)
    }
}
