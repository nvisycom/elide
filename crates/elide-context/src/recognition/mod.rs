//! [`Enhanced`]: a [`Recognizer`] decorator that keyword-boosts another
//! recognizer's entities.
//!
//! A recognizer emits located [`Entity`]s, each carrying the
//! [`recognized_range`] it was found at in the recognized text. [`Enhanced`]
//! runs the inner recognizer, then runs the keyword-boost [`Enhancer`] over
//! those entities — lifting confidence where a context keyword fires in the
//! word window around each entity's range (or in an out-of-band hint) — and
//! records a refinement event per boost. Because it reads only the
//! modality-free fields, the same `Enhanced<R>` serves every modality.
//!
//! [`Recognizer`]: elide_core::recognition::Recognizer
//! [`Entity`]: elide_core::entity::Entity
//! [`recognized_range`]: elide_core::entity::Entity::recognized_range
//! [`Enhancer`]: crate::Enhancer

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::entity::provenance::Event;
use elide_core::modality::TextRecognizable;
use elide_core::recognition::{Recognizer, RecognizerContext, RecognizerId};

use crate::io::Tokens;
use crate::{Context, Enhancer};

/// Wraps a [`Recognizer`] with a keyword-boost [`Enhancer`] applied to its
/// entities.
///
/// This is where context enhancement happens: the inner recognizer produces
/// located entities (each with its [`recognized_range`]), the enhancer lifts
/// confidence where a keyword fires near an entity, and a refinement event is
/// recorded per boost. An [`Enhancer`] with no rules is the no-op case.
///
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`recognized_range`]: elide_core::entity::Entity::recognized_range
/// [`Enhancer`]: crate::Enhancer
pub struct Enhanced<R> {
    inner: R,
    enhancer: Enhancer,
}

impl<R> Enhanced<R> {
    /// Wrap `inner` with a keyword-boost `enhancer`.
    ///
    /// An [`Enhancer`] with no rules is the "no enhancement" case: the inner
    /// recognizer's entities pass through unchanged.
    pub fn new(inner: R, enhancer: Enhancer) -> Self {
        Self { inner, enhancer }
    }

    /// Borrow the wrapped recognizer.
    pub fn inner(&self) -> &R {
        &self.inner
    }
}

#[async_trait::async_trait]
impl<M, R> Recognizer<M> for Enhanced<R>
where
    M: TextRecognizable,
    R: Recognizer<M> + 'static,
{
    fn id(&self) -> RecognizerId {
        self.inner.id()
    }

    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        let mut entities = self.inner.recognize(data, ctx).await?;
        if self.enhancer.is_empty() {
            return Ok(entities);
        }

        let text = M::as_text(data, &ctx.artifacts);
        // A hint is a text annotation (a header, a field name). Read each
        // through the modality's text view; for text/tabular that is the
        // hint's own payload.
        let hint_texts: Vec<&str> = ctx
            .context_hints
            .iter()
            .map(|h| M::as_text(&h.data, &ctx.artifacts))
            .collect();
        let mut context = Context::new(text).with_hints(&hint_texts);
        if let Some(tokens) = ctx.artifacts.get::<Tokens>() {
            context = context.with_tokens(tokens.as_slice());
        }
        if let Some(language) = ctx.primary_language() {
            context = context.with_language(language);
        }

        let boosts = self.enhancer.enhance(&mut entities, &context);
        for boost in boosts {
            let hint = boost.hint_index.map(|i| ctx.context_hints[i].clone());
            // Where the boosting keyword sits: a hint carries its own
            // location; an in-text match resolves its keyword range through
            // the modality (a pixel box / time span), mirroring how the entity
            // itself was located. `None` when it can't be placed.
            let location = match (&hint, boost.keyword_range) {
                (Some(h), _) => Some(h.location.clone()),
                (None, Some(range)) => M::locate(range, data, &ctx.artifacts),
                (None, None) => None,
            };
            let entity = &mut entities[boost.entity_index];
            let label = entity.label.clone();
            entity.provenance.record(
                Event::refinement(
                    boost.source,
                    boost.before,
                    boost.after,
                    boost.keyword,
                    hint,
                    location,
                )
                .with_reason(format!(
                    "context keyword near `{}` (+{:.3})",
                    label.as_str(),
                    boost.amount,
                )),
            );
        }
        Ok(entities)
    }
}
