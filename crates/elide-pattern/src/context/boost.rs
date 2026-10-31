//! The context pass: lift each match whose rule's keywords or anchor are
//! found near it.

use std::ops::Range;

use elide_core::modality::text::Token;
use elide_core::primitive::{Confidence, LanguageTag};
use hipstr::HipStr;

use super::compiled::CompiledContext;
use super::matching::{KeywordMatcher, is_whole_word};
use super::window::{slice_tokens_around, token_span, word_window};
use crate::recognizer::RawMatch;

/// Words searched before a match for its keywords and anchors.
const PREFIX_WORDS: usize = 5;

/// Words searched after a match. Equal to [`PREFIX_WORDS`], so trailing
/// context ("123-45-6789 (social security)") counts the same as leading.
const SUFFIX_WORDS: usize = 5;

/// Audit source of a lift by a keyword in the text around the match.
const SOURCE_WINDOW: &str = "context";

/// Audit source of a lift by a keyword in an out-of-band hint.
const SOURCE_HINT: &str = "context-hint";

/// Audit source of a lift by a neighbouring anchor match.
const SOURCE_ANCHOR: &str = "context-anchor";

/// What context reads around the matches of one subject.
pub(crate) struct Surroundings<'a> {
    /// The recognized text the matches' ranges index into.
    pub(crate) text: &'a str,
    /// The tokens a tokenizing enricher produced for `text`, if any.
    pub(crate) tokens: Option<&'a [Token]>,
    /// The call's asserted languages; empty activates every language's list.
    pub(crate) languages: &'a [&'a LanguageTag],
    /// Out-of-band hint texts, each with its index into the subject's hints.
    pub(crate) hints: &'a [(usize, &'a str)],
}

/// One lift applied to a match, for the recognizer to record on its entity.
pub(crate) struct Boost {
    /// Index of the lifted match.
    pub(crate) target: usize,
    /// Audit source of the lift.
    pub(crate) source: &'static str,
    /// The match's confidence after the lift.
    pub(crate) confidence: Confidence,
    /// The text that fired: a keyword occurrence, or the anchor's label.
    pub(crate) keyword: HipStr<'static>,
    /// The subject hint the keyword was found in.
    pub(crate) hint: Option<usize>,
    /// Where in the recognized text the keyword was found.
    pub(crate) keyword_range: Option<Range<usize>>,
}

/// Where a keyword fired for one match.
struct Found {
    source: &'static str,
    keyword: HipStr<'static>,
    hint: Option<usize>,
    keyword_range: Option<Range<usize>>,
}

/// Lift every match in `matches` its rule's context supports, and return each
/// lift in the order applied.
///
/// Keywords run first, so a match's confidence reflects its own context before
/// it serves as another match's anchor. Anchors then run to a fixpoint, since
/// they can chain: a number lifts a currency beside it, which then clears the
/// floor to lift that number. Each match is lifted at most once by each.
pub(crate) fn boost(
    matches: &mut [RawMatch],
    contexts: &[CompiledContext],
    matcher: &dyn KeywordMatcher,
    around: &Surroundings<'_>,
) -> Vec<Boost> {
    let mut boosts = Vec::new();

    for (target, m) in matches.iter_mut().enumerate() {
        let Some(context) = m.context.map(|i| &contexts[i]) else {
            continue;
        };
        let Some(found) = find_keyword(context, &m.range, matcher, around) else {
            continue;
        };
        let lifted = m.confidence.saturating_add(context.boost.get());
        if lifted == m.confidence {
            continue;
        }
        m.confidence = lifted;
        boosts.push(Boost {
            target,
            source: found.source,
            confidence: lifted,
            keyword: found.keyword,
            hint: found.hint,
            keyword_range: found.keyword_range,
        });
    }

    let mut anchored = vec![false; matches.len()];
    loop {
        let neighbours: Vec<_> = matches
            .iter()
            .map(|m| (m.label.clone(), m.range.clone(), m.confidence))
            .collect();
        let mut changed = false;
        for (target, m) in matches.iter_mut().enumerate() {
            let Some(context) = m.context.map(|i| &contexts[i]) else {
                continue;
            };
            let Some(anchor) = &context.anchor else {
                continue;
            };
            if anchored[target] {
                continue;
            }
            let window = window_bounds(&m.range, around);
            let found = neighbours
                .iter()
                .enumerate()
                .any(|(other, (label, span, confidence))| {
                    other != target
                        && *label == anchor.label
                        && *confidence >= anchor.min_confidence
                        && window.start <= span.start
                        && span.end <= window.end
                });
            if !found {
                continue;
            }
            anchored[target] = true;
            let lifted = m.confidence.saturating_add(context.boost.get());
            if lifted == m.confidence {
                continue;
            }
            m.confidence = lifted;
            changed = true;
            boosts.push(Boost {
                target,
                source: SOURCE_ANCHOR,
                confidence: lifted,
                keyword: anchor.label.as_str().into(),
                hint: None,
                keyword_range: None,
            });
        }
        if !changed {
            break;
        }
    }

    boosts
}

/// The first of `context`'s active keywords found in the words around
/// `range`, or failing that in a hint.
fn find_keyword(
    context: &CompiledContext,
    range: &Range<usize>,
    matcher: &dyn KeywordMatcher,
    around: &Surroundings<'_>,
) -> Option<Found> {
    let (window, tokens, offset) = window(range, around);
    let in_text = context
        .active_keywords(around.languages)
        .filter_map(|keywords| first_hit(context, matcher, window, tokens, keywords))
        .min_by_key(|hit| hit.start);
    if let Some(hit) = in_text {
        return Some(Found {
            source: SOURCE_WINDOW,
            keyword: HipStr::from(&window[hit.clone()]),
            hint: None,
            keyword_range: Some(offset + hit.start..offset + hit.end),
        });
    }

    around.hints.iter().find_map(|&(hint, text)| {
        let hit = context
            .active_keywords(around.languages)
            .find_map(|keywords| first_hit(context, matcher, text, &[], keywords))?;
        Some(Found {
            source: SOURCE_HINT,
            keyword: HipStr::from(&text[hit]),
            hint: Some(hint),
            keyword_range: None,
        })
    })
}

/// The first occurrence of `keywords` in `text` that satisfies the rule's
/// word-boundary policy.
fn first_hit(
    context: &CompiledContext,
    matcher: &dyn KeywordMatcher,
    text: &str,
    tokens: &[Token],
    keywords: &[HipStr<'static>],
) -> Option<Range<usize>> {
    matcher
        .matches(text, tokens, keywords)
        .into_iter()
        .find(|hit| !context.word_boundary || is_whole_word(text, hit))
}

/// The text around `range` searched for keywords, the tokens in it, and its
/// start in the recognized text. Uses the token stream when tokens reach the
/// match, and Unicode word segmentation of the text otherwise.
fn window<'a>(range: &Range<usize>, around: &Surroundings<'a>) -> (&'a str, &'a [Token], usize) {
    let tokens = around
        .tokens
        .map(|tokens| slice_tokens_around(tokens, range.clone(), PREFIX_WORDS, SUFFIX_WORDS))
        .unwrap_or_default();
    if tokens.is_empty() {
        let (text, offset) = word_window(around.text, range.clone(), PREFIX_WORDS, SUFFIX_WORDS);
        (text, &[], offset)
    } else {
        let (text, offset) = token_span(around.text, tokens, range.clone());
        (text, tokens, offset)
    }
}

/// The byte range of the window around `range`, for testing whether an anchor
/// neighbour falls inside it.
fn window_bounds(range: &Range<usize>, around: &Surroundings<'_>) -> Range<usize> {
    let (text, _, offset) = window(range, around);
    offset..offset + text.len()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use elide_core::entity::LabelRef;
    use elide_core::entity::audit::PatternEvent;

    use super::*;
    use crate::context::matching::SubstringMatcher;
    use crate::context::{Anchor, RuleContext};

    fn keywords(terms: &[&str]) -> RuleContext {
        RuleContext::from(terms.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>())
    }

    fn anchored(label: &str, min_confidence: f32) -> RuleContext {
        RuleContext {
            anchor: Some(Anchor {
                label: label.to_owned(),
                min_confidence: Some(min_confidence),
            }),
            ..RuleContext::default()
        }
    }

    fn compile(declared: &RuleContext) -> CompiledContext {
        CompiledContext::compile(declared, &[]).expect("context declares something")
    }

    fn raw(label: &str, range: Range<usize>, score: f32, context: Option<usize>) -> RawMatch {
        let score = Confidence::new(score).unwrap();
        RawMatch {
            label: LabelRef::new(label),
            score,
            confidence: score,
            range,
            pattern: PatternEvent::default(),
            context,
        }
    }

    fn run(text: &str, matches: &mut [RawMatch], contexts: &[CompiledContext]) -> Vec<Boost> {
        run_with(text, &[], matches, contexts)
    }

    fn run_with(
        text: &str,
        hints: &[(usize, &str)],
        matches: &mut [RawMatch],
        contexts: &[CompiledContext],
    ) -> Vec<Boost> {
        let around = Surroundings {
            text,
            tokens: None,
            languages: &[],
            hints,
        };
        boost(matches, contexts, &SubstringMatcher, &around)
    }

    #[test]
    fn a_keyword_in_the_window_lifts_the_match() {
        let contexts = [compile(&keywords(&["social security"]))];
        //          0         1         2
        //          012345678901234567890123456
        let text = "Social Security 123-45-6789";
        let mut matches = [raw("ssn", 16..27, 0.5, Some(0))];
        let boosts = run(text, &mut matches, &contexts);

        assert_eq!(matches[0].confidence, Confidence::new(0.85).unwrap());
        assert_eq!(boosts.len(), 1);
        assert_eq!(boosts[0].source, SOURCE_WINDOW);
        // The text that fired, as written, and where it sits.
        assert_eq!(boosts[0].keyword, "Social Security");
        assert_eq!(boosts[0].keyword_range, Some(0..15));
    }

    #[test]
    fn no_keyword_leaves_the_match_alone() {
        let contexts = [compile(&keywords(&["social security"]))];
        let mut matches = [raw("ssn", 14..25, 0.5, Some(0))];
        let boosts = run("the number is 123-45-6789", &mut matches, &contexts);

        assert_eq!(matches[0].confidence, Confidence::new(0.5).unwrap());
        assert!(boosts.is_empty());
    }

    #[test]
    fn a_rules_keywords_lift_only_its_own_matches() {
        // Two rules emit the same label; only the first declares "ssn".
        let contexts = [compile(&keywords(&["ssn"]))];
        let text = "SSN 123-45-6789 or 987-65-4329";
        let mut matches = [
            raw("government_id", 4..15, 0.5, Some(0)),
            raw("government_id", 19..30, 0.5, None),
        ];
        run(text, &mut matches, &contexts);

        assert!(matches[0].confidence.get() > 0.5);
        assert_eq!(matches[1].confidence, Confidence::new(0.5).unwrap());
    }

    #[test]
    fn a_match_is_lifted_once_however_many_keywords_fire() {
        let mut lists = HashMap::new();
        lists.insert("en".parse().unwrap(), vec!["card".to_owned()]);
        lists.insert("es".parse().unwrap(), vec!["tarjeta".to_owned()]);
        let contexts = [compile(&RuleContext {
            languages: lists,
            ..RuleContext::default()
        })];
        let mut matches = [raw("payment_card", 13..17, 0.3, Some(0))];
        let boosts = run("card tarjeta 4111", &mut matches, &contexts);

        assert_eq!(boosts.len(), 1);
        assert_eq!(matches[0].confidence, Confidence::new(0.65).unwrap());
    }

    #[test]
    fn a_keyword_in_a_hint_lifts_the_match() {
        let contexts = [compile(&keywords(&["ssn"]))];
        let mut matches = [raw("ssn", 0..11, 0.5, Some(0))];
        // The subject's first hint had no text; the index still points at the
        // second.
        let boosts = run_with("123-45-6789", &[(1, "ssn")], &mut matches, &contexts);

        assert_eq!(matches[0].confidence, Confidence::new(0.85).unwrap());
        assert_eq!(boosts[0].source, SOURCE_HINT);
        assert_eq!(boosts[0].hint, Some(1));
        assert!(boosts[0].keyword_range.is_none());
    }

    #[test]
    fn an_anchor_in_the_window_lifts_the_match() {
        let contexts = [compile(&anchored("currency", 0.35))];
        let mut matches = [
            raw("monetary_amount", 0..4, 0.1, Some(0)),
            raw("currency", 5..8, 0.5, None),
        ];
        let boosts = run("5000 USD ok", &mut matches, &contexts);

        assert!(matches[0].confidence.get() > 0.1);
        assert_eq!(matches[1].confidence, Confidence::new(0.5).unwrap());
        assert_eq!(boosts[0].source, SOURCE_ANCHOR);
        assert_eq!(boosts[0].keyword, "currency");
    }

    #[test]
    fn an_anchor_below_its_floor_does_not_lift() {
        let contexts = [compile(&anchored("currency", 0.35))];
        let mut matches = [
            raw("monetary_amount", 0..4, 0.1, Some(0)),
            raw("currency", 5..8, 0.3, None),
        ];
        run("2010 ADA xx", &mut matches, &contexts);

        assert_eq!(matches[0].confidence, Confidence::new(0.1).unwrap());
    }

    #[test]
    fn anchors_chain_to_a_fixpoint() {
        // A weak number lifts the currency (low floor), which then clears the
        // number's floor and lifts it in turn.
        let contexts = [
            compile(&anchored("currency", 0.35)),
            compile(&anchored("monetary_amount", 0.1)),
        ];
        let mut matches = [
            raw("monetary_amount", 0..4, 0.1, Some(0)),
            raw("currency", 5..8, 0.3, Some(1)),
        ];
        run("5000 USD ok", &mut matches, &contexts);

        assert!(matches[1].confidence.get() > 0.3);
        assert!(matches[0].confidence.get() > 0.1);
    }
}
