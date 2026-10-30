//! The walk back from an anchor, and the prose trimmed from the front of the
//! name it finds.

use std::ops::Range;

use super::compiled::{CompiledWalk, MAX_LOWERCASE_RUN};
use super::lexicon::Bridge;
use super::scan::{self, Kind, Step};

impl CompiledWalk {
    /// The start of the name ending at `anchor`, its lead-in prose trimmed, or
    /// `None` when no name precedes the anchor or it numbers a document part.
    pub(super) fn before(&self, text: &str, anchor: usize, bridge: &Bridge<'_>) -> Option<usize> {
        let start = self.walk_back(text, anchor, bridge)?;
        self.trim_front(text, start, anchor)
    }

    /// The leftmost capitalized word or number of the name ending at
    /// `anchor`, or `None` when none precedes it.
    fn walk_back(&self, text: &str, anchor: usize, bridge: &Bridge<'_>) -> Option<usize> {
        let words = &self.lexicon;
        let mut pos = anchor;
        let mut start = None;
        let mut capital = false;
        let mut lowercase_run = 0;
        // A connective or comma lies between the next word and the name.
        let mut linked = false;

        for step_index in 0..self.max_words {
            let Some(Step {
                range,
                comma,
                quoted,
            }) = scan::step_back(text, pos, step_index == 0)
            else {
                break;
            };
            let word = &text[range.clone()];
            if scan::ends_sentence(word) {
                break;
            }
            match scan::kind(word) {
                None | Some(Kind::Amount) => break,
                Some(Kind::Capital) => {
                    // "Acme GmbH and Beta GmbH": an earlier name ends here.
                    if (linked || comma) && self.anchors.ends_name(word) {
                        break;
                    }
                    capital = true;
                    start = Some(range.start);
                    lowercase_run = 0;
                    linked = false;
                }
                Some(Kind::Number) => {
                    start = Some(range.start);
                    linked = false;
                }
                Some(Kind::Ampersand) => linked = true,
                Some(Kind::Lower) => {
                    let lower = word.to_lowercase();
                    let party = words.party_connectors.contains(&lower);
                    if capital && party {
                        if self.joins_parties(text, range.start, pos..anchor) {
                            break;
                        }
                    } else if capital && !bridge.allows(&lower) {
                        break;
                    }
                    if !party && !words.connectors.contains(&lower) {
                        lowercase_run += 1;
                        if lowercase_run > MAX_LOWERCASE_RUN {
                            break;
                        }
                    }
                    linked = true;
                }
            }
            pos = range.start;
            if quoted {
                break;
            }
        }
        start
    }

    /// Whether the party connector at `at` joins two parties rather than two
    /// words of one name, `next` being the name after it: it follows a comma
    /// ("…, and Acme LLC"), a role or document noun ("the Company and Acme
    /// LLC"), or a person with a middle initial ("John A. Smith and Acme LLC")
    /// unless a firm noun follows ("John A. Smith and Sons Ltd").
    fn joins_parties(&self, text: &str, at: usize, next: Range<usize>) -> bool {
        let words = &self.lexicon;
        let after_role = scan::step_back(text, at, false)
            .is_some_and(|prev| words.introduces(&text[prev.range]));
        let firm = scan::words(text, next)
            .next()
            .is_some_and(|next| words.firm_nouns.contains(&text[next].to_lowercase()));
        let after_person = scan::middle_initial_before(text, at) && !firm;
        scan::comma_before(text, at) || after_role || after_person
    }

    /// Trim the prose a walk passed from the front of the name at
    /// `start..anchor`: a clause phrase, a sentence verb, a leading role. `None`
    /// when no name is left, or the name numbers a document part.
    fn trim_front(&self, text: &str, mut start: usize, anchor: usize) -> Option<usize> {
        let words = &self.lexicon;
        let span = &text[start..anchor];
        let mut cut: Option<usize> = None;
        for clause in &words.clauses {
            if let Some(end) = last_phrase_end(span, clause) {
                cut = cut.max(Some(end));
            }
        }
        for range in scan::words(text, start..anchor) {
            let word = &text[range.clone()];
            if word.starts_with(char::is_lowercase) && words.verbs.contains(&word.to_lowercase()) {
                cut = cut.max(Some(range.end - start));
            }
        }
        if let Some(cut) = cut {
            start = scan::words(text, start + cut..anchor)
                .find(|w| {
                    let word = &text[w.clone()];
                    scan::kind(word) == Some(Kind::Capital) && !words.introduces(word)
                })?
                .start;
        }

        // A leading role ("Seller Acme GmbH") goes, unless the name would be
        // left without a capitalized word ("Société générale SA").
        let ranges: Vec<Range<usize>> = scan::words(text, start..anchor).collect();
        let lowered: Vec<String> = ranges
            .iter()
            .map(|w| text[w.clone()].to_lowercase())
            .collect();
        let capital_from = |skip: usize| {
            ranges[skip..]
                .iter()
                .any(|w| scan::kind(&text[w.clone()]) == Some(Kind::Capital))
        };
        let role = words
            .roles
            .iter()
            .filter(|role| lowered.len() > role.len() && lowered.starts_with(role))
            .map(Vec::len)
            .filter(|&skip| capital_from(skip))
            .max();
        if let Some(skip) = role {
            start = ranges[skip].start;
        }

        let rest: Vec<&str> = scan::words(text, start..anchor).map(|w| &text[w]).collect();
        if let [first, marker, ..] = rest.as_slice()
            && words.structural.contains(&first.to_lowercase())
            && is_marker(marker)
        {
            return None;
        }
        Some(start)
    }
}

/// Where the last whole-word occurrence of `phrase` in `span` ends.
fn last_phrase_end(span: &str, phrase: &str) -> Option<usize> {
    span.match_indices(phrase)
        .filter(|(i, _)| {
            let before = span[..*i].chars().next_back();
            let after = span[i + phrase.len()..].chars().next();
            before.is_none_or(|c| !scan::is_word_char(c))
                && after.is_none_or(|c| !scan::is_word_char(c))
        })
        .map(|(i, _)| i + phrase.len())
        .last()
}

/// A one-letter or letter-number marker that numbers a document part: `A`,
/// `A-1`, `2`.
fn is_marker(word: &str) -> bool {
    word.chars().count() <= 3
        && word
            .chars()
            .all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '-')
}
