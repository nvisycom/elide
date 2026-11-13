//! The two scans of a recognition pass: every regex, and every dictionary term.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::Range;

use elide_core::entity::LabelRef;
use elide_core::primitive::Confidence;
use elide_core::recognition::{Context, Languages};

use super::raw::is_whole_token;
use super::{PatternRecognizer, RawMatch};
use crate::validators::ValidationContext;

/// One dictionary's hit, before the longest per position is kept.
struct DictionaryHit {
    dictionary: usize,
    range: Range<usize>,
    score: Confidence,
}

impl PatternRecognizer {
    /// Every pattern match in `text`, validated.
    pub(super) fn scan_patterns(
        &self,
        text: &str,
        languages: &Languages<'_>,
        ctx: &Context<'_>,
    ) -> Vec<RawMatch> {
        let Some(set) = self.regex_set.as_ref() else {
            return Vec::new();
        };
        let validation = ValidationContext {
            countries: ctx.scope().countries.clone(),
            language: languages.primary().cloned(),
        };
        let mut matches = Vec::new();
        for pattern_id in set.matches(text).into_iter() {
            let pat = &self.patterns[pattern_id];
            let Some(label) = pat.scope.label(languages, ctx) else {
                continue;
            };
            for m in pat.spans(text) {
                if let Some(validator) = pat.validator.as_ref()
                    && !validator.validate(m.as_str(), &validation)
                {
                    continue;
                }
                matches.push(pat.raw_match(label.clone(), m.range()));
            }
        }
        matches
    }

    /// Every dictionary match in `text`.
    ///
    /// Each dictionary keeps, of its own hits that pass its filters, the
    /// longest at each position, so a longer hit another dictionary owns, or
    /// one this dictionary rejects as not a whole word, never hides a shorter
    /// valid one.
    pub(super) fn scan_dictionaries(
        &self,
        text: &str,
        languages: &Languages<'_>,
        ctx: &Context<'_>,
    ) -> Vec<RawMatch> {
        let Some(terms) = self.terms.as_ref() else {
            return Vec::new();
        };
        let labels: Vec<Option<&LabelRef>> = self
            .dictionaries
            .iter()
            .map(|d| d.scope.label(languages, ctx))
            .collect();
        if labels.iter().all(Option::is_none) {
            return Vec::new();
        }

        // Only the longest hit per dictionary and start can be kept, so nested
        // overlaps (a dictionary of `a`, `a a`, `a a a`, … over a run of `a`s)
        // never pile up: at most one candidate per dictionary per position.
        let mut hits: Vec<DictionaryHit> = Vec::new();
        let mut longest: HashMap<(usize, usize), usize> = HashMap::new();
        for mat in terms.automaton.find_overlapping_iter(text) {
            for owner in &terms.owners[mat.pattern().as_usize()] {
                if labels[owner.dictionary].is_none() {
                    continue;
                }
                let range = mat.range();
                if self.dictionaries[owner.dictionary].word_boundary
                    && !is_whole_token(text, range.clone())
                {
                    continue;
                }
                let hit = DictionaryHit {
                    dictionary: owner.dictionary,
                    range,
                    score: owner.score,
                };
                match longest.entry((hit.dictionary, hit.range.start)) {
                    Entry::Vacant(slot) => {
                        slot.insert(hits.len());
                        hits.push(hit);
                    }
                    Entry::Occupied(slot) => {
                        let kept = &mut hits[*slot.get()];
                        if hit.range.end > kept.range.end {
                            *kept = hit;
                        }
                    }
                }
            }
        }

        hits.sort_by_key(|hit| (hit.dictionary, hit.range.start));
        let mut kept: Vec<DictionaryHit> = Vec::with_capacity(hits.len());
        for hit in hits {
            if let Some(last) = kept.last()
                && last.dictionary == hit.dictionary
                && hit.range.start < last.range.end
            {
                continue;
            }
            kept.push(hit);
        }
        kept.sort_by_key(|hit| (hit.range.start, hit.dictionary));
        kept.into_iter()
            .filter_map(|hit| {
                let label = labels[hit.dictionary]?;
                let dict = &self.dictionaries[hit.dictionary];
                Some(dict.raw_match(label.clone(), hit.score, hit.range))
            })
            .collect()
    }
}
