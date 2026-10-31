//! [`CompiledDictionary`]: one dictionary's slice of the recognizer's shared
//! Aho-Corasick automaton.

use std::ops::Range;

use elide_core::entity::LabelRef;
use elide_core::entity::audit::PatternEvent;
use elide_core::primitive::{Confidence, CountryCode, LanguageTag};

use crate::recognizer::RawMatch;

/// Source of truth for one runtime dictionary: its term range
/// inside the shared Aho-Corasick automaton, plus per-dictionary
/// emission metadata.
pub(crate) struct CompiledDictionary {
    pub name: String,
    /// Candidate labels, most-specific first. The recognizer resolves one
    /// against the request catalog before emitting.
    pub labels: Vec<LabelRef>,
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
    /// The dictionary's context, as an index into the recognizer's
    /// contexts; `None` when it declares none.
    pub context: Option<usize>,
}

impl CompiledDictionary {
    /// A [`RawMatch`] for an Aho-Corasick dictionary hit at `range` under the
    /// resolved `label` (the first candidate the request catalog declared) with
    /// the resolved per-term `score`, for the recognizer to place.
    pub(crate) fn raw_match(
        &self,
        label: LabelRef,
        score: Confidence,
        range: Range<usize>,
    ) -> RawMatch {
        RawMatch {
            label,
            score,
            confidence: score,
            range,
            context: self.context,
            pattern: PatternEvent {
                name: self.name.clone().into(),
                contextual: false,
                ..PatternEvent::default()
            },
        }
    }
}
