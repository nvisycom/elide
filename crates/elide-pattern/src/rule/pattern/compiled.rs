//! [`CompiledPattern`]: one regex variant, compiled for the recognizer.

use std::ops::Range;
use std::sync::Arc;

use elide_core::entity::LabelRef;
use elide_core::entity::audit::PatternRule;
use elide_core::primitive::Confidence;
use regex::{Match, Regex};

use crate::recognizer::RawMatch;
use crate::rule::RuleScope;
use crate::validators::Validator;

/// One compiled regex slot: a single `(pattern, variant)` pair,
/// keyed in the shared `RegexSet` by its position in
/// `PatternRecognizer.patterns`. Pattern-level metadata (labels,
/// languages, provenance) is repeated across the pattern's variants so
/// the dispatch loop has everything it needs without a second
/// indirection.
pub(crate) struct CompiledPattern {
    /// The parent pattern's labels and locale scope.
    pub scope: RuleScope,
    pub regex: Regex,
    /// Index of the regex's [`ENTITY_GROUP`] capture group, when it defines
    /// one: that group, not the whole match, is the entity's span.
    ///
    /// [`ENTITY_GROUP`]: super::Variant::ENTITY_GROUP
    pub span_group: Option<usize>,
    pub score: Confidence,
    pub validator: Option<Arc<dyn Validator>>,
    /// The parent pattern's context, as an index into the recognizer's
    /// contexts; `None` when it declares none.
    pub context: Option<usize>,
    /// The provenance recorded on every match: the pattern's name, the
    /// variant's regex, and its validator's name.
    pub rule: PatternRule,
}

impl CompiledPattern {
    /// Each match's entity span in `text`: the [`span_group`] when the regex
    /// defines one, otherwise the whole match. A match whose group did not
    /// participate, or matched nothing, yields no span.
    ///
    /// [`span_group`]: Self::span_group
    pub(crate) fn spans<'t>(&'t self, text: &'t str) -> impl Iterator<Item = Match<'t>> + 't {
        let (whole, grouped) = match self.span_group {
            None => (Some(self.regex.find_iter(text)), None),
            Some(group) => {
                let spans = self
                    .regex
                    .captures_iter(text)
                    .filter_map(move |caps| caps.get(group))
                    .filter(|m| !m.is_empty());
                (None, Some(spans))
            }
        };
        whole
            .into_iter()
            .flatten()
            .chain(grouped.into_iter().flatten())
    }

    /// A [`RawMatch`] for a regex hit at `range` under the resolved `label`
    /// (the first candidate the request catalog declared), for the recognizer
    /// to place.
    pub(crate) fn raw_match(&self, label: LabelRef, range: Range<usize>) -> RawMatch {
        RawMatch {
            label,
            score: self.score,
            confidence: self.score,
            range,
            context: self.context,
            rule: self.rule.clone(),
        }
    }
}
