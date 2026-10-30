//! [`CompiledWalk`]: a [`Walk`] compiled for the recognizer.

use std::collections::HashSet;
use std::ops::Range;

use aho_corasick::{AhoCorasick, MatchKind};
use elide_core::entity::LabelRef;
use elide_core::entity::audit::PatternEvent;
use elide_core::primitive::{Confidence, CountryCode, LanguageTag};
use elide_core::{Error, ErrorKind, Result};

use super::lexicon::{Bridge, Lexicon};
use super::scan::{self, Kind};
use super::{Direction, Walk};
use crate::recognizer::RawMatch;

/// Lowercase words a walk passes in a row before it gives up: enough for a
/// name's inner words ("Základní škola a Mateřská škola"), not for a sentence.
pub(super) const MAX_LOWERCASE_RUN: usize = 4;

/// A walk compiled for the recognizer: its anchors in one automaton, and its
/// words merged across languages.
pub(crate) struct CompiledWalk {
    pub name: String,
    /// Candidate labels, most-specific first.
    pub labels: Vec<LabelRef>,
    pub languages: Vec<LanguageTag>,
    pub countries: Vec<CountryCode>,
    /// The walk's context, as an index into the recognizer's contexts.
    pub context: Option<usize>,
    pub(super) anchors: Anchors,
    pub(super) lexicon: Lexicon,
    direction: Direction,
    include_anchor: bool,
    pub(super) max_words: usize,
}

/// A walk's anchors: the automaton that finds them, and the two ways a walk
/// meets one again.
pub(super) struct Anchors {
    automaton: Option<AhoCorasick>,
    /// Score per anchor, indexed by the automaton's pattern id.
    scores: Vec<Confidence>,
    /// Abbreviated anchors, by [`scan::anchor_key`]: one reached across a
    /// connective or comma ends a backward walk ("Acme GmbH and Beta GmbH").
    endings: HashSet<String>,
    /// Single-word anchors, by [`scan::anchor_key`]: one reached by a forward
    /// walk may start another name.
    words: HashSet<String>,
    /// The distinctive ones among `words`, which are not also initials or name
    /// words the way ambiguous ones are (`M.`, `Don`).
    distinctive: HashSet<String>,
}

impl Anchors {
    fn new(walk: &Walk) -> Result<Self> {
        let automaton = if walk.anchors.is_empty() {
            None
        } else {
            let automaton = AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .build(walk.anchors.iter().map(|t| t.term.as_str()))
                .map_err(|e| {
                    Error::new(
                        ErrorKind::Configuration,
                        format!("walk `{}`: compiling anchors: {e}", walk.name),
                    )
                })?;
            Some(automaton)
        };
        let single_words = || {
            walk.anchors
                .iter()
                .filter(|t| !t.term.contains(char::is_whitespace))
        };
        Ok(Self {
            automaton,
            scores: walk
                .anchors
                .iter()
                .map(|t| t.score.unwrap_or(walk.score))
                .collect(),
            endings: single_words()
                .filter(|t| is_abbreviation(&t.term))
                .map(|t| scan::anchor_key(&t.term))
                .collect(),
            words: single_words().map(|t| scan::anchor_key(&t.term)).collect(),
            distinctive: single_words()
                .filter(|t| t.score.is_none_or(|score| score >= walk.score))
                .map(|t| scan::anchor_key(&t.term))
                .collect(),
        })
    }

    /// Whether `word` ends an earlier name when a backward walk meets it.
    pub(super) fn ends_name(&self, word: &str) -> bool {
        self.endings.contains(&scan::anchor_key(word))
    }

    /// Whether `word` is a single-word anchor.
    pub(super) fn is_anchor(&self, word: &str) -> bool {
        self.words.contains(&scan::anchor_key(word))
    }

    /// Whether `word` is a distinctive single-word anchor.
    pub(super) fn is_distinctive(&self, word: &str) -> bool {
        self.distinctive.contains(&scan::anchor_key(word))
    }
}

impl CompiledWalk {
    /// Compile `walk`, its context already compiled at index `context`.
    pub(crate) fn compile(walk: &Walk, context: Option<usize>) -> Result<Self> {
        if walk.direction == Direction::Both && !walk.include_anchor {
            return Err(Error::new(
                ErrorKind::Configuration,
                format!(
                    "walk `{}`: a walk in both directions keeps its anchor inside the name",
                    walk.name
                ),
            ));
        }
        Ok(Self {
            name: walk.name.clone(),
            labels: walk.labels.clone(),
            languages: walk.languages.clone(),
            countries: walk.countries.clone(),
            context,
            anchors: Anchors::new(walk)?,
            lexicon: Lexicon::new(&walk.words),
            direction: walk.direction,
            include_anchor: walk.include_anchor,
            max_words: walk.max_words,
        })
    }

    /// Every name in `text`, with its score. `languages` pick which lowercase
    /// words a name may contain.
    pub(crate) fn find(
        &self,
        text: &str,
        languages: &[&LanguageTag],
    ) -> Vec<(Range<usize>, Confidence)> {
        let Some(automaton) = &self.anchors.automaton else {
            return Vec::new();
        };
        let bridge = self.lexicon.bridge(languages);
        let mut names: Vec<(Range<usize>, Confidence)> = automaton
            .find_iter(text)
            .filter(|hit| scan::opens(text, hit.start()) && scan::closes(text, hit.end()))
            .filter_map(|hit| {
                let name = self.name_around(text, hit.range(), &bridge)?;
                Some((name, self.anchors.scores[hit.pattern().as_usize()]))
            })
            .collect();

        // A name inside a longer one is part of it: `Goldman Sachs & Co.` in
        // `Goldman Sachs & Co. LLC`.
        names.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
        let mut kept: Vec<(Range<usize>, Confidence)> = Vec::with_capacity(names.len());
        for name in names {
            let inside = kept
                .iter()
                .any(|(outer, _)| outer.start <= name.0.start && name.0.end <= outer.end);
            if !inside {
                kept.push(name);
            }
        }
        kept
    }

    /// A [`RawMatch`] for a name at `range` under the resolved `label`.
    pub(crate) fn raw_match(
        &self,
        label: LabelRef,
        range: Range<usize>,
        score: Confidence,
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

    /// The name around the anchor at `anchor`, or `None` when the words next
    /// to it do not make one.
    fn name_around(
        &self,
        text: &str,
        anchor: Range<usize>,
        bridge: &Bridge<'_>,
    ) -> Option<Range<usize>> {
        let (before, after) = match self.direction {
            Direction::Backward => (Some(self.before(text, anchor.start, bridge)?), None),
            Direction::Forward => (None, Some(self.after(text, anchor.end, bridge)?)),
            Direction::Both => (
                self.before(text, anchor.start, bridge),
                self.after(text, anchor.end, bridge),
            ),
        };
        let before = before.map(|start| start..anchor.start);
        if !self.has_name_word(text, before.clone(), after.clone()) {
            return None;
        }

        if !self.include_anchor {
            // A one-sided walk: the words on that side, without the anchor or
            // the space and comma before it.
            return after.or_else(|| {
                let before = before?;
                let end = text[..before.end].trim_end_matches([' ', '\t', ',']).len();
                Some(before.start..end)
            });
        }
        let start = before.map_or(anchor.start, |before| before.start);
        let end = after.map_or(anchor.end, |after| after.end);
        Some(start..end)
    }

    /// Whether the words before and after the anchor hold a capitalized word
    /// that names something on its own, rather than a generic one ("Audit
    /// Committee", "Board of Directors").
    fn has_name_word(
        &self,
        text: &str,
        before: Option<Range<usize>>,
        after: Option<Range<usize>>,
    ) -> bool {
        let generic = &self.lexicon.generic;
        let generic_before = &self.lexicon.generic_before;
        let names = |range: Option<Range<usize>>, is_before: bool| {
            range
                .into_iter()
                .flat_map(|range| scan::words(text, range))
                .any(|word| {
                    let word = &text[word];
                    let key = scan::word_key(word);
                    scan::kind(word) == Some(Kind::Capital)
                        && !generic.contains(&key)
                        && !(is_before && generic_before.contains(&key))
                })
        };
        names(before, true) || names(after, false)
    }
}

/// Whether an anchor also ends a name when met again inside a walk: an
/// abbreviation (`Ltd`, `GmbH`, `S.A.`), not an ordinary word (`Bank`).
fn is_abbreviation(term: &str) -> bool {
    let letters = term.chars().filter(|c| c.is_alphabetic()).count();
    let upper = term.chars().filter(|c| c.is_uppercase()).count();
    let mixed = upper >= 2 && upper < letters;
    term.contains('.') || mixed || letters <= 3
}
