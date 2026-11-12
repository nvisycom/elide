//! [`CompiledDictionary`] and [`DictionaryTerms`]: dictionaries compiled into
//! the recognizer's shared Aho-Corasick automaton.

use std::ops::Range;

use aho_corasick::AhoCorasick;
use elide_core::entity::LabelRef;
use elide_core::entity::audit::PatternRule;
use elide_core::primitive::Confidence;

use crate::recognizer::RawMatch;
use crate::rule::RuleScope;

/// One dictionary's emission metadata, for the recognizer's shared
/// Aho-Corasick automaton. Which of the automaton's terms belong to it, and at
/// what score, is kept per term ([`TermOwner`]), since one term can belong to
/// several dictionaries.
pub(crate) struct CompiledDictionary {
    /// The dictionary's labels and locale scope.
    pub scope: RuleScope,
    /// Reject matches whose immediate neighbours are word
    /// characters (alphanumeric or `_`). Mirrors regex `\b`.
    pub word_boundary: bool,
    /// The dictionary's context, as an index into the recognizer's
    /// contexts; `None` when it declares none.
    pub context: Option<usize>,
    /// The provenance recorded on every match: the dictionary's name.
    pub rule: PatternRule,
}

/// Every distinct dictionary term in one automaton, and the dictionaries
/// holding each, indexed by the automaton's pattern id.
pub(crate) struct DictionaryTerms {
    pub automaton: AhoCorasick,
    pub owners: Vec<Vec<TermOwner>>,
}

/// A dictionary that holds one of the automaton's terms, and the term's
/// confidence there, resolved at build time from the dictionary's `scoring`
/// policy and any per-term override.
pub(crate) struct TermOwner {
    /// Index of the dictionary in the recognizer's dictionaries.
    pub dictionary: usize,
    pub score: Confidence,
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
            rule: self.rule.clone(),
        }
    }
}
