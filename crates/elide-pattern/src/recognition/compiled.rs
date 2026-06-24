//! Compiled, recognizer-ready forms of [`Regex`] rules and
//! [`Dictionary`]s.
//!
//! [`PatternRecognizerBuilder::build`] compiles each regex variant
//! into a [`::regex::Regex`] and folds every dictionary's terms
//! into a shared [`AhoCorasick`] automaton, then stores the
//! per-rule emission metadata next to those scanners. This module
//! holds the per-rule metadata structs ([`CompiledPattern`],
//! [`CompiledDictionary`]) and their `draft` constructors — the bits that
//! turn a regex / Aho-Corasick hit into an [`EntityDraft`] (stream-positioned;
//! the `Enhanced` adapter lifts it to an entity).
//!
//! [`Regex`]: super::Regex
//! [`Dictionary`]: super::Dictionary
//! [`AhoCorasick`]: aho_corasick::AhoCorasick
//! [`EntityDraft`]: elide_context::EntityDraft
//! [`PatternRecognizerBuilder::build`]: super::PatternRecognizerBuilder::build

use std::ops::Range;
use std::sync::Arc;

use elide_core::entity::LabelRef;
use elide_core::entity::provenance::PatternEvent;
use elide_core::primitive::{Confidence, CountryCode, LanguageTag};
use regex::Regex;

use crate::validators::Validator;

/// One pattern/dictionary hit before placement: the recognizer lifts it to a
/// located [`Entity`] via `M::locate(range)`, keeping `range` as the entity's
/// `recognized_range`.
///
/// [`Entity`]: elide_core::entity::Entity
pub(super) struct RawMatch {
    pub label: LabelRef,
    pub confidence: Confidence,
    pub range: Range<usize>,
    pub pattern: PatternEvent,
    pub reason: String,
}

/// One compiled regex slot: a single `(pattern, variant)` pair,
/// keyed in the shared `RegexSet` by its position in
/// `PatternRecognizer.patterns`. Pattern-level metadata (name,
/// label, languages) is repeated across the pattern's variants so
/// the dispatch loop has everything it needs without a second
/// indirection.
///
/// `context` is intentionally not stored on compiled state — the
/// recognizer's wrapping `Enhanced` layer harvests keywords from the source
/// patterns at build time.
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
    /// A [`RawMatch`] for a regex hit at `range`, for the recognizer to place.
    pub(super) fn raw_match(&self, range: Range<usize>) -> RawMatch {
        RawMatch {
            label: self.label.clone(),
            confidence: self.score,
            range,
            pattern: PatternEvent {
                name: self.pattern_name.clone().into(),
                regex: Some(self.regex.as_str().into()),
                validator: self
                    .validator
                    .as_ref()
                    .map(|_| self.pattern_name.clone().into()),
                contextual: false,
            },
            reason: format!("pattern `{}` matched", self.pattern_name),
        }
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
    /// A [`RawMatch`] for an Aho-Corasick dictionary hit at `range` with the
    /// resolved per-term `score`, for the recognizer to place.
    pub(super) fn raw_match(&self, score: Confidence, range: Range<usize>) -> RawMatch {
        RawMatch {
            label: self.label.clone(),
            confidence: score,
            range,
            pattern: PatternEvent {
                name: self.name.clone().into(),
                contextual: false,
                ..PatternEvent::default()
            },
            reason: format!("dictionary `{}` matched", self.name),
        }
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
