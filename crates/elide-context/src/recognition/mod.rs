//! Stream recognition: the seam that lets a text-stream recognizer be
//! context-enhanced for any modality.
//!
//! A [`StreamRecognizer`] finds matches in the recognized-text stream and
//! returns [`EntityDraft`]s carrying a *stream* byte range. [`Enhanced`]
//! wraps one as a full [`Recognizer`]: it runs the optional keyword-boost
//! [`Enhancer`] over the drafts (where the stream range is still available)
//! and then [`lift`]s each draft to a located [`Entity`] via the modality's
//! [`locate`]. The stream range is consumed at lift and never reaches the
//! entity, so enhancement works uniformly for text, tabular, image, and
//! audio without the entity carrying an engine-specific offset.
//!
//! [`Recognizer`]: elide_core::recognition::Recognizer
//! [`Entity`]: elide_core::entity::Entity
//! [`Enhancer`]: crate::Enhancer
//! [`locate`]: elide_core::modality::TextRecognizable::locate

mod draft;
mod lift;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::entity::provenance::Event;
use elide_core::modality::TextRecognizable;
use elide_core::recognition::{Recognizer, RecognizerContext, RecognizerId};

pub use self::draft::{DraftEvent, EntityDraft};
pub use self::lift::{lift, lift_all};
use crate::io::Tokens;
use crate::{Context, Enhancer};

/// Finds matches in the recognized-text stream, as stream-positioned drafts.
///
/// Returns [`EntityDraft`]s carrying a stream byte range, not yet placed in
/// the medium.
///
/// A narrow, opt-in complement to [`Recognizer`] — not a replacement. A
/// stream recognizer does all of its work over the `&str` that
/// [`as_text`] exposes (matching, validation, filtering), producing drafts
/// with a stream byte range; wrapping it in [`Enhanced`] yields a full
/// `Recognizer<M>`. Modalities whose text is a projection (image OCR, audio
/// STT) are supported because [`as_text`] reads their enrichment artifact.
///
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`as_text`]: elide_core::modality::TextRecognizable::as_text
pub trait StreamRecognizer<M: TextRecognizable>: Send + Sync {
    /// This recognizer's identity (name + version).
    fn id(&self) -> RecognizerId;

    /// Find matches in `text` (the recognized-text stream) and return them
    /// as stream-positioned drafts.
    fn find(&self, text: &str, ctx: &RecognizerContext<'_, M>) -> Vec<EntityDraft>;
}

/// Adapts a [`StreamRecognizer`] into a full [`Recognizer`].
///
/// Optionally runs a keyword-boost [`Enhancer`] over the drafts before
/// lifting them.
///
/// This is where context enhancement happens: `find` produces drafts that
/// still hold their stream range, the enhancer lifts confidence where a
/// keyword fires (operating on the drafts), and each draft is then lifted to
/// a located entity. Because everything before lift is modality-free, the
/// same `Enhanced<R>` serves every modality.
///
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`Enhancer`]: crate::Enhancer
pub struct Enhanced<R> {
    inner: R,
    enhancer: Enhancer,
}

impl<R> Enhanced<R> {
    /// Wrap `inner` with a keyword-boost `enhancer` applied before lift.
    ///
    /// An [`Enhancer`] with no rules is the "no enhancement" case: drafts
    /// are lifted as-is.
    pub fn new(inner: R, enhancer: Enhancer) -> Self {
        Self { inner, enhancer }
    }

    /// Borrow the wrapped stream recognizer.
    pub fn inner(&self) -> &R {
        &self.inner
    }
}

impl<M, R> Recognizer<M> for Enhanced<R>
where
    M: TextRecognizable,
    R: StreamRecognizer<M> + 'static,
{
    fn id(&self) -> RecognizerId {
        self.inner.id()
    }

    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        let text = M::as_text(data, &ctx.artifacts);
        let mut drafts = self.inner.find(text, ctx);

        // Enhance the drafts while the stream range is still available. An
        // enhancer with no rules is the no-op case — skip the context setup.
        let boosts = if self.enhancer.is_empty() {
            Vec::new()
        } else {
            // A hint is a text annotation (a header, a field name). Read
            // each through the modality's text view; for text/tabular
            // that is the hint's own payload.
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
            self.enhancer.enhance(&mut drafts, &context)
        };

        // Lift each draft to a located entity, dropping the unplaceable, and
        // record the boost refinements that fired on the lifted entities.
        let mut entities: Vec<Entity<M>> = Vec::with_capacity(drafts.len());
        let mut draft_to_entity = vec![None; drafts.len()];
        for (draft_index, draft) in drafts.into_iter().enumerate() {
            let before = draft.confidence;
            if let Some(entity) = lift::<M>(draft, data, &ctx.artifacts) {
                draft_to_entity[draft_index] = Some((entities.len(), before));
                entities.push(entity);
            }
        }
        for boost in boosts {
            let Some((entity_index, _)) = draft_to_entity[boost.entity_index] else {
                continue; // the boosted draft didn't survive lift
            };
            let hint = boost.hint_index.map(|i| ctx.context_hints[i].clone());
            // Where the boosting keyword sits: a hint carries its own
            // location; an in-text match resolves its stream range through
            // the modality (a pixel box / time span), mirroring how the
            // entity itself was located. `None` when it can't be placed.
            let location = match (&hint, boost.keyword_range) {
                (Some(h), _) => Some(h.location.clone()),
                (None, Some(range)) => M::locate(range, data, &ctx.artifacts),
                (None, None) => None,
            };
            let label = entities[entity_index].label.clone();
            entities[entity_index].provenance.record(
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
