//! [`RecognizerLanguage`]: the language-aware surface of a
//! [`RecognizerContext`].

use crate::modality::Modality;
use crate::primitive::{Confidence, Language, LanguageTag};
use crate::recognition::RecognizerContext;

/// The language-aware surface of a [`RecognizerContext`].
///
/// Groups every "what languages does this call concern?" operation in one
/// trait so recognizers and the context enhancer consult the call's
/// languages through a single, named contract instead of poking at the
/// underlying list.
pub trait RecognizerLanguage {
    /// Add a caller-asserted language to the call.
    fn assert_language(&mut self, language: LanguageTag, confidence: Option<Confidence>);

    /// The call's languages, ranked best-first.
    ///
    /// Sorted by confidence descending (a missing confidence ranks last),
    /// with an asserted language breaking ties ahead of a detected one.
    /// Empty when the call has no language information.
    fn languages(&self) -> Vec<&Language>;

    /// The single most likely language tag for this call, or `None` when
    /// no language is known.
    fn primary_language(&self) -> Option<&LanguageTag>;

    /// Whether a recognizer rule scoped to `allowed` languages should run
    /// for this call.
    ///
    /// - An empty `allowed` list means the rule is language-agnostic and
    ///   always runs.
    /// - Otherwise the rule runs when *any* of the call's languages shares
    ///   a primary subtag with an entry in `allowed` (so an `["en"]` rule
    ///   fires when the call includes `"en-US"`).
    /// - When the call has no languages, the rule still runs: we can't
    ///   disprove applicability without information.
    fn applies_to_language(&self, allowed: &[LanguageTag]) -> bool;
}

impl<M: Modality> RecognizerLanguage for RecognizerContext<M> {
    fn assert_language(&mut self, language: LanguageTag, confidence: Option<Confidence>) {
        self.languages
            .push(Language::asserted(language, confidence));
    }

    fn languages(&self) -> Vec<&Language> {
        self.languages.ranked()
    }

    fn primary_language(&self) -> Option<&LanguageTag> {
        self.languages.best().map(|d| &d.language)
    }

    fn applies_to_language(&self, allowed: &[LanguageTag]) -> bool {
        if allowed.is_empty() {
            return true;
        }
        if self.languages.is_empty() {
            return true;
        }
        self.languages
            .as_slice()
            .iter()
            .any(|d| allowed.iter().any(|a| a.matches(&d.language)))
    }
}
