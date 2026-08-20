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
use elide_core::entity::audit::AuditEvent;
use elide_core::modality::TextRecognizable;
use elide_core::primitive::LanguageTag;
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, RecognizerId};

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
    ) -> Result<Recognition<M>> {
        let recognition = self.inner.recognize(data, ctx).await?;
        #[cfg(feature = "usage")]
        let model_usage = recognition.model_usage;
        let mut entities = recognition.entities;
        if self.enhancer.is_empty() {
            let recognition = Recognition::new(entities);
            #[cfg(feature = "usage")]
            let recognition = match model_usage {
                Some(model_usage) => recognition.with_model_usage(model_usage),
                None => recognition,
            };
            return Ok(recognition);
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
        // Only *asserted* languages select which per-language context fires; a
        // *detected* language does not. Detection is unreliable on the short,
        // word-poor chunks a codec emits (a lone `Card: 4111…` cell resolves to
        // an arbitrary language), and a misdetected chunk language would
        // deactivate the very context whose keyword sits in the text. With no
        // asserted language the list is empty, which the enhancer reads as
        // permissive — every per-language context is active, so the keyword
        // that actually appears (`card`, `tarjeta`, `Kreditkarte`, …) fires
        // regardless of the surrounding language. Activating a language's
        // context is harmless when its keyword is absent.
        let languages: Vec<&LanguageTag> = ctx.asserted_languages();
        let mut context = Context::new(text)
            .with_hints(&hint_texts)
            .with_languages(&languages);
        if let Some(tokens) = ctx.artifacts.get::<Tokens>() {
            context = context.with_tokens(tokens.as_slice());
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
            entity.audit.record(AuditEvent::refinement(
                boost.source,
                boost.confidence,
                boost.keyword,
                hint,
                location,
            ));
        }
        let recognition = Recognition::new(entities);
        #[cfg(feature = "usage")]
        let recognition = match model_usage {
            Some(model_usage) => recognition.with_model_usage(model_usage),
            None => recognition,
        };
        Ok(recognition)
    }
}
