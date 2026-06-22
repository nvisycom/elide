//! Compiled, recognizer-ready forms of [`Regex`] rules and
//! [`Dictionary`]s.
//!
//! [`PatternRecognizerBuilder::build`] compiles each regex variant
//! into a [`::regex::Regex`] and folds every dictionary's terms
//! into a shared [`AhoCorasick`] automaton, then stores the
//! per-rule emission metadata next to those scanners. This module
//! holds the per-rule metadata structs ([`CompiledPattern`],
//! [`CompiledDictionary`]) and their `build_entity` constructors —
//! the bits that turn a regex / Aho-Corasick hit into an
//! `Entity<Text>`.
//!
//! [`Regex`]: super::Regex
//! [`Dictionary`]: super::Dictionary
//! [`AhoCorasick`]: aho_corasick::AhoCorasick
//! [`PatternRecognizerBuilder::build`]: super::PatternRecognizerBuilder::build

use std::ops::Range;
use std::sync::Arc;

use elide_core::entity::provenance::{Event, PatternEvent};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::TextRecognizable;
use elide_core::primitive::{Confidence, CountryCode, LanguageTag};
use elide_core::recognition::RecognizerContext;
use regex::Regex;

use crate::validators::Validator;

/// One compiled regex slot: a single `(pattern, variant)` pair,
/// keyed in the shared `RegexSet` by its position in
/// `PatternRecognizer.patterns`. Pattern-level metadata (name,
/// label, languages) is repeated across the pattern's variants so
/// the dispatch loop has everything it needs without a second
/// indirection.
///
/// `context` is intentionally not stored on compiled state — the
/// recognizer's wrapping `ContextEnhanced` layer harvests keywords from
/// the source patterns at build time.
pub(super) struct CompiledPattern {
    /// Pattern name (e.g. `"ssn"`). Surfaced in trail provenance.
    pub pattern_name: String,
    pub label: LabelRef,
    pub regex: Regex,
    pub score: Confidence,
    pub validator: Option<Arc<dyn Validator>>,
    /// Languages the parent pattern applies to.
    /// Empty means "any language".
    pub languages: Vec<LanguageTag>,
    /// Countries the parent pattern applies to.
    /// Empty means "any country".
    pub countries: Vec<CountryCode>,
}

impl CompiledPattern {
    /// Emit an `Entity<M>` for a regex match at `[start, end)` in
    /// chunk-local byte coordinates. The recognizer phase lifts the
    /// location to absolute document coordinates after dispatch.
    pub(super) fn build_entity<M: TextRecognizable>(
        &self,
        range: Range<usize>,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Entity<M> {
        let location = M::locate(range, data, ctx);
        let event = Event::pattern(
            "pattern",
            self.score,
            location.clone(),
            PatternEvent {
                name: self.pattern_name.clone().into(),
                regex: Some(self.regex.as_str().into()),
                validator: self
                    .validator
                    .as_ref()
                    .map(|_| self.pattern_name.clone().into()),
                contextual: false,
            },
        )
        .with_reason(format!("pattern `{}` matched", self.pattern_name));
        Entity::builder()
            .with_label(self.label.clone())
            .with_location(location)
            .with_confidence(self.score)
            .with_event(event)
            .build()
            .expect("required fields provided")
    }
}

/// Source of truth for one runtime dictionary: its term range
/// inside the shared Aho-Corasick automaton, plus per-dictionary
/// emission metadata.
pub(super) struct CompiledDictionary {
    pub name: String,
    pub label: LabelRef,
    /// First term-id (inclusive) for this dictionary inside the
    /// shared automaton.
    pub term_start: usize,
    /// One past the last term-id for this dictionary inside the
    /// shared automaton.
    pub term_end: usize,
    /// Per-term confidence, indexed by `term_id - term_start`.
    /// Resolved at compile time from the dictionary's `scoring`
    /// policy and any per-term overrides.
    pub term_scores: Vec<Confidence>,
    /// Languages this dictionary applies to. Empty means "any
    /// language".
    pub languages: Vec<LanguageTag>,
    /// Countries this dictionary applies to. Empty means "any
    /// country".
    pub countries: Vec<CountryCode>,
    /// Reject matches whose immediate neighbours are word
    /// characters (alphanumeric or `_`). Mirrors regex `\b`.
    pub word_boundary: bool,
}

impl CompiledDictionary {
    /// Emit an `Entity<M>` for an Aho-Corasick hit at `[start, end)`
    /// in chunk-local byte coordinates. `score` is the per-term
    /// confidence resolved at recognizer-build time (the dictionary's
    /// `scoring` policy or per-term override).
    pub(super) fn build_entity<M: TextRecognizable>(
        &self,
        score: Confidence,
        range: Range<usize>,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Entity<M> {
        let location = M::locate(range, data, ctx);
        let event = Event::pattern(
            "pattern",
            score,
            location.clone(),
            PatternEvent {
                name: self.name.clone().into(),
                contextual: false,
                ..PatternEvent::default()
            },
        )
        .with_reason(format!("dictionary `{}` matched", self.name));
        Entity::builder()
            .with_label(self.label.clone())
            .with_location(location)
            .with_confidence(score)
            .with_event(event)
            .build()
            .expect("required fields provided")
    }
}

/// Mirror of regex `\b` for the byte range `text[start..end]`:
/// the immediate neighbour characters (or start/end of input)
/// must not be word characters. A word character here is Unicode
/// alphanumeric or `_`, matching the conventional regex
/// definition.
///
/// Operates on `char` boundaries, not raw bytes, so multibyte
/// codepoints don't trigger false rejections (`é` is one char,
/// not two).
pub(super) fn has_word_boundaries(text: &str, range: Range<usize>) -> bool {
    let left_is_word = text[..range.start]
        .chars()
        .next_back()
        .is_some_and(is_word_char);
    let right_is_word = text[range.end..].chars().next().is_some_and(is_word_char);
    !left_is_word && !right_is_word
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_word_boundaries_handles_edges_and_unicode() {
        // Match touches both edges of the input → boundaries OK.
        assert!(has_word_boundaries("hello", 0..5));
        // Match preceded by a word char → not a boundary.
        assert!(!has_word_boundaries("example", 5..7));
        // Match followed by a word char → not a boundary.
        assert!(!has_word_boundaries("amount", 0..2));
        // Space surround → boundaries OK.
        assert!(has_word_boundaries(" am ", 1..3));
        // Unicode word char on the left → not a boundary.
        assert!(!has_word_boundaries("café_am", 5..7));
        // Punctuation around → boundaries OK.
        assert!(has_word_boundaries("(am)", 1..3));
    }
}
