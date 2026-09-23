//! [`Languages`]: a borrowed view over a call's asserted + detected languages.

use crate::entity::Entity;
use crate::modality::Modality;
use crate::primitive::{LanguageClaim, LanguageTag};

/// The languages in play for one recognition call: the caller-asserted ones
/// (from the scope) and the ones a detector found (on the subject), viewed
/// together.
///
/// A borrowed view built by
/// [`RecognizerContext::languages`](super::RecognizerContext::languages); it
/// owns nothing, just the two slices, and folds them for the queries a
/// recognizer runs — which language is primary, whether a language-scoped rule
/// applies, attributing an entity to the language of its span.
///
/// The asserted / detected distinction is deliberate: detection is unreliable on
/// short, word-poor input, so a *detected* language never hard-filters a rule
/// (only [`asserted_apply_to`](Self::asserted_apply_to) does), while ranking and
/// span attribution use both. A caller assertion is full confidence, so it sorts
/// ahead of any detection.
pub struct Languages<'a> {
    asserted: &'a [LanguageClaim],
    detected: &'a [LanguageClaim],
}

impl<'a> Languages<'a> {
    /// View over the `asserted` (scope) and `detected` (subject) claims.
    pub(super) fn new(asserted: &'a [LanguageClaim], detected: &'a [LanguageClaim]) -> Self {
        Self { asserted, detected }
    }

    /// Every claim for the call, asserted then detected, in stored order.
    fn all(&self) -> impl Iterator<Item = &'a LanguageClaim> {
        self.asserted.iter().chain(self.detected)
    }

    /// The caller-asserted language tags only, excluding anything a detector
    /// added. Empty when the caller asserted no language (read as "any").
    ///
    /// Selecting which per-language context to activate keys on this, so a
    /// misdetected chunk language can't deactivate the context whose keyword
    /// actually sits in the text.
    #[must_use]
    pub fn asserted(&self) -> Vec<&'a LanguageTag> {
        self.asserted.iter().map(|c| &c.language).collect()
    }

    /// Whether the caller asserted any language.
    #[must_use]
    pub fn any_asserted(&self) -> bool {
        !self.asserted.is_empty()
    }

    /// The call's claims ranked best-first by confidence. A caller assertion is
    /// full confidence, so it sorts ahead of any detection; detections order by
    /// their scores. Empty when the call has no language information.
    #[must_use]
    pub fn ranked(&self) -> Vec<&'a LanguageClaim> {
        let mut all: Vec<&LanguageClaim> = self.all().collect();
        all.sort_by(|a, b| b.confidence().get().total_cmp(&a.confidence().get()));
        all
    }

    /// The single most likely language tag, or `None` when none is known.
    #[must_use]
    pub fn primary(&self) -> Option<&'a LanguageTag> {
        self.ranked().first().map(|c| &c.language)
    }

    /// Whether a rule scoped to `allowed` languages should run, considering
    /// **all** the call's languages (asserted or detected).
    ///
    /// - An empty `allowed` list means the rule is language-agnostic: always runs.
    /// - Otherwise it runs when any of the call's languages shares a primary
    ///   subtag with an entry in `allowed` (so an `["en"]` rule fires on `en-US`).
    /// - When the call has no languages, it still runs — applicability can't be
    ///   disproven without information.
    #[must_use]
    pub fn apply_to(&self, allowed: &[LanguageTag]) -> bool {
        matches_any(self.all(), allowed)
    }

    /// Whether a rule scoped to `allowed` languages should run, filtering
    /// **only** on caller-asserted languages.
    ///
    /// The locale hard filter: a language-scoped pattern is suppressed when the
    /// caller asserted a language and none of the asserted languages match the
    /// rule's scope (declaring a document Spanish stops the German/Italian/…
    /// locale patterns firing on same-shaped values). A *detected* language never
    /// participates here — detection is too unreliable to suppress a valid match.
    /// Always runs for a language-agnostic rule (empty `allowed`) or when no
    /// language is asserted.
    #[must_use]
    pub fn asserted_apply_to(&self, allowed: &[LanguageTag]) -> bool {
        matches_any(self.asserted.iter(), allowed)
    }

    /// Stamp each entity's [`language`] from the call's detected-language spans:
    /// match the entity's [`recognized_range`] against the [`LanguageClaim`] whose
    /// span covers it.
    ///
    /// A span-less claim (whole-payload scope) applies to any range, so a
    /// monolingual document attributes every entity to its single language.
    /// Entities with no `recognized_range` (a natively-located VLM box) are left
    /// untouched, as is the whole set when no language is known.
    ///
    /// [`language`]: crate::entity::Entity::language
    /// [`recognized_range`]: crate::entity::Entity::recognized_range
    pub fn stamp<M: Modality>(&self, entities: &mut [Entity<M>]) {
        let ranked = self.ranked();
        if ranked.is_empty() {
            return;
        }
        for entity in entities.iter_mut() {
            let Some(range) = entity.recognized_range.clone() else {
                continue;
            };
            let resolved = ranked.iter().find(|claim| {
                claim
                    .span
                    .as_ref()
                    .is_none_or(|span| range.start >= span.start && range.end <= span.end)
            });
            if let Some(claim) = resolved {
                entity.language = Some(claim.language.clone());
            }
        }
    }
}

/// Whether any claim's language shares a primary subtag with an `allowed` entry.
///
/// An empty `allowed` (language-agnostic rule) or an empty claim iterator
/// (nothing to disprove applicability) both return `true`.
fn matches_any<'a>(
    claims: impl Iterator<Item = &'a LanguageClaim>,
    allowed: &[LanguageTag],
) -> bool {
    if allowed.is_empty() {
        return true;
    }
    let mut claims = claims.peekable();
    if claims.peek().is_none() {
        return true;
    }
    claims.any(|c| allowed.iter().any(|a| a.matches(&c.language)))
}
