//! [`Languages`]: a borrowed view over a call's asserted + detected languages.

use crate::entity::Entity;
use crate::modality::Modality;
use crate::primitive::{LanguageClaim, LanguageTag};

/// The languages in play for one recognition call: the caller-asserted ones
/// (from the scope) and the ones a detector found (on the subject), viewed
/// together.
///
/// A borrowed view built by
/// [`Context::languages`](super::Context::languages); it
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

    /// Stamp each entity's [`language`] by matching its [`recognized_range`]
    /// against the [`LanguageClaim`] whose span covers it.
    ///
    /// When the call carries any *spanned* claim (per-region detection ran, as it
    /// does whenever more than one language is in play), attribution is by
    /// coverage: an entity gets the language of the best-ranked claim whose span
    /// contains its range, and is left unset when no span covers it — a
    /// linguistically-empty entity, such as a bare account number between two
    /// asserted languages, has no language to attribute rather than defaulting to
    /// whichever was asserted first. A span-less claim (whole-payload scope) is
    /// used only when there are *no* spanned claims at all — the monolingual
    /// case, where every entity is that one language.
    ///
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
        // A span-less claim (whole-document scope) is only ever an asserted one;
        // detections always carry a span. When the caller asserted two or more
        // languages, detection ran to tell them apart, so a span-less asserted
        // claim must not act as a catch-all fallback — otherwise every entity
        // would collapse onto the first-ranked language. Attribution is then
        // strictly by coverage: an entity no detected span covers is left unset (a
        // bare account number between two languages has no language of its own).
        // With zero or one asserted language the span-less claim is the correct
        // monolingual fallback.
        let coverage_only = self.asserted.len() >= 2;
        for entity in entities.iter_mut() {
            let Some(range) = entity.recognized_range.clone() else {
                continue;
            };
            let resolved = ranked.iter().find(|claim| match claim.span.as_ref() {
                Some(span) => range.start >= span.start && range.end <= span.end,
                None => !coverage_only,
            });
            entity.language = resolved.map(|claim| claim.language.clone());
        }
    }
}

#[cfg(all(test, feature = "fixtures"))]
mod tests {
    use std::ops::Range;

    use super::*;
    use crate::modality::text::Text;
    use crate::primitive::{Confidence, LanguageClaim, LanguageTag};

    fn tag(s: &str) -> LanguageTag {
        s.parse().unwrap()
    }

    fn entity(label: &str, range: Range<usize>) -> Entity<Text> {
        let mut e = Entity::<Text>::fixture(label, (range.start, range.end));
        e.recognized_range = Some(range);
        e
    }

    fn lang_of(e: &Entity<Text>) -> Option<String> {
        e.language.as_ref().map(|l| l.primary_subtag().to_owned())
    }

    #[test]
    fn single_asserted_language_stamps_every_entity() {
        // One asserted language is monolingual: no detection, span-less claim is
        // the whole-document fallback, so even a language-less digit run gets it.
        let asserted = [LanguageClaim::asserted(tag("en"))];
        let view = Languages::new(&asserted, &[]);
        let mut entities = vec![entity("x", 0..5), entity("y", 10..14)];
        view.stamp(&mut entities);
        assert_eq!(lang_of(&entities[0]).as_deref(), Some("en"));
        assert_eq!(lang_of(&entities[1]).as_deref(), Some("en"));
    }

    #[test]
    fn multiple_asserted_attribute_by_detected_span_not_first() {
        // Two asserted languages: detection ran and tagged regions. Each entity
        // takes the language of the span that covers it, not the first asserted.
        let asserted = [
            LanguageClaim::asserted(tag("pl")),
            LanguageClaim::asserted(tag("en")),
        ];
        let detected = [
            LanguageClaim::detected(tag("pl"), Confidence::new(0.9).unwrap()).with_span(0..20),
            LanguageClaim::detected(tag("en"), Confidence::new(0.9).unwrap()).with_span(20..40),
        ];
        let view = Languages::new(&asserted, &detected);
        let mut entities = vec![entity("in_pl", 2..8), entity("in_en", 25..30)];
        view.stamp(&mut entities);
        assert_eq!(lang_of(&entities[0]).as_deref(), Some("pl"));
        assert_eq!(lang_of(&entities[1]).as_deref(), Some("en"));
    }

    #[test]
    fn multiple_asserted_leave_uncovered_entity_unset() {
        // A language-less entity (no detected span covers it) between two asserted
        // languages is left unset rather than defaulting to the first asserted.
        let asserted = [
            LanguageClaim::asserted(tag("pl")),
            LanguageClaim::asserted(tag("en")),
        ];
        let detected =
            [LanguageClaim::detected(tag("en"), Confidence::new(0.9).unwrap()).with_span(0..10)];
        let view = Languages::new(&asserted, &detected);
        let mut entities = vec![entity("covered", 2..8), entity("bare_digits", 50..66)];
        view.stamp(&mut entities);
        assert_eq!(lang_of(&entities[0]).as_deref(), Some("en"));
        assert_eq!(lang_of(&entities[1]), None);
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
