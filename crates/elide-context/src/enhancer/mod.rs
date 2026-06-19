//! [`Enhancer`]: post-recognition keyword-boost pass for any
//! [`Entity<Text>`] regardless of which recognizer produced it.

use std::collections::HashMap;

use elide_core::entity::provenance::Event;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::TextBacked;

use crate::io::Token;
use crate::matching::KeywordMatcher;
use crate::rule::BoostRule;

mod context;
mod window;

pub use self::context::Context;
use self::window::{slice_tokens_around, token_span, word_window};

/// Source name stamped onto refinement events the enhancer records when
/// the in-text word window fires.
const EVENT_SOURCE_WINDOW: &str = "context";

/// Source name stamped onto refinement events the enhancer records when
/// an out-of-band hint fires.
const EVENT_SOURCE_HINT: &str = "context-hint";

/// Post-recognition keyword-boost pass over recognized entities.
///
/// Holds a label-keyed [`BoostRule`] map plus the keyword-matching
/// strategy, and lifts the confidence of each text entity whose
/// label has a rule and whose surrounding word window contains one
/// of the rule's keywords.
///
/// Construct via [`Enhancer::new`]. Rules are passed in by value;
/// duplicates for the same label are merged via
/// [`BoostRule::merge`] (union of keywords; window radii / `boost`
/// kept from the first-seen rule).
///
/// The matcher defaults are picked by the engine that constructs
/// the enhancer: [`SubstringMatcher`] when no upstream NLP engine
/// produces tokens, [`LemmaMatcher`] when one does.
///
/// [`SubstringMatcher`]: crate::matching::SubstringMatcher
/// [`LemmaMatcher`]: crate::matching::LemmaMatcher
pub struct Enhancer {
    /// Rules bucketed by label. Within one bucket, each entry is
    /// a distinct `(language)` scope; rules sharing the same
    /// `(label, language)` are pre-merged via [`BoostRule::merge`]
    /// at construction. Per-entity application looks up the
    /// bucket once by label, then walks the small inner vec
    /// filtering on the per-call language hint.
    rules: HashMap<LabelRef, Vec<BoostRule>>,
    matcher: Box<dyn KeywordMatcher>,
}

impl Enhancer {
    /// Construct from a rule iterator and matcher. Rules sharing
    /// the same `(label, language)` are merged via
    /// [`BoostRule::merge`]; rules with the same label but
    /// distinct languages live as separate entries inside the
    /// label's bucket.
    ///
    /// `matcher` is any concrete [`KeywordMatcher`] taken by value;
    /// it is boxed internally, so callers don't wrap it themselves.
    pub fn new<M: KeywordMatcher + 'static>(
        rules: impl IntoIterator<Item = BoostRule>,
        matcher: M,
    ) -> Self {
        Self::with_boxed_matcher(rules, Box::new(matcher))
    }

    /// Construct from a rule iterator and an already-boxed matcher.
    ///
    /// Use this when the matcher is selected at runtime (e.g. a
    /// substring vs. lemma strategy chosen by whether an NLP engine
    /// produced tokens) and is therefore already a trait object.
    /// [`new`] is the by-value convenience that wraps for you.
    ///
    /// [`new`]: Self::new
    pub fn with_boxed_matcher(
        rules: impl IntoIterator<Item = BoostRule>,
        matcher: Box<dyn KeywordMatcher>,
    ) -> Self {
        let mut buckets: HashMap<LabelRef, Vec<BoostRule>> = HashMap::new();
        for rule in rules {
            let bucket = buckets.entry(rule.label.clone()).or_default();
            if let Some(existing) = bucket.iter_mut().find(|r| r.language == rule.language) {
                existing.merge(rule);
            } else {
                bucket.push(rule);
            }
        }
        Self {
            rules: buckets,
            matcher,
        }
    }

    /// `true` when no rules are registered. Engine code uses this
    /// to short-circuit calls to [`enhance`] entirely.
    ///
    /// [`enhance`]: Self::enhance
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Number of distinct labels with rules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Apply boost rules to `entities` in place. For each entity:
    /// walk every rule registered for its label whose language
    /// scope applies under `ctx.language`, walk a window of
    /// `prefix_words` words before and `suffix_words` words after
    /// the entity's location, ask the matcher whether any keyword
    /// fires, and on a hit lift confidence by the rule's `boost`
    /// (saturating at the [`Confidence`] ceiling) plus record a
    /// refinement [`Event`] in the entity's provenance.
    ///
    /// The in-text and hint paths are independent: at most one
    /// boost per rule fires per entity (window first, hint as
    /// fallback) so a rule with a long keyword list can't
    /// double-dip.
    ///
    /// [`Confidence`]: elide_core::primitive::Confidence
    pub fn enhance<M: TextBacked>(&self, entities: &mut [Entity<M>], ctx: &Context<'_>) {
        if self.rules.is_empty() {
            return;
        }
        for entity in entities {
            self.enhance_one(entity, ctx);
        }
    }

    fn enhance_one<M: TextBacked>(&self, entity: &mut Entity<M>, ctx: &Context<'_>) {
        let Some(bucket) = self.rules.get(&entity.label) else {
            return;
        };
        for rule in bucket {
            if !rule.applies_to_language(ctx.language) {
                continue;
            }
            if rule.keywords.is_empty() {
                continue;
            }
            self.apply_rule(entity, rule, ctx);
        }
    }

    fn apply_rule<M: TextBacked>(
        &self,
        entity: &mut Entity<M>,
        rule: &BoostRule,
        ctx: &Context<'_>,
    ) {
        // The entity is still chunk-local here; its location spans a byte
        // range of `ctx.text` (the chunk payload).
        let span = M::span(&entity.location);
        let start = span.start;
        let end = span.end;

        // Prefer the token stream when the producer reached this
        // entity. Fall back to the word-segmented substring window
        // whenever the token slice would be empty; that covers
        // `tokens: None`, `tokens: Some(&[])`, and the "tokens
        // present but none overlap the entity" case (e.g. NLP
        // engine only tokenized part of the document).
        let token_slice = ctx
            .tokens
            .map(|toks| slice_tokens_around(toks, start, end, rule.prefix_words, rule.suffix_words))
            .unwrap_or(&[]);
        let (snippet, tokens_in_window): (&str, &[Token]) = if token_slice.is_empty() {
            let snippet = word_window(ctx.text, start, end, rule.prefix_words, rule.suffix_words);
            (snippet, &[])
        } else {
            let snippet = token_span(ctx.text, token_slice, start, end);
            (snippet, token_slice)
        };

        let matched = if self
            .matcher
            .any_match(snippet, tokens_in_window, &rule.keywords)
        {
            (EVENT_SOURCE_WINDOW, false)
        } else if ctx
            .hints
            .iter()
            .any(|h| self.matcher.any_match(h, &[], &rule.keywords))
        {
            (EVENT_SOURCE_HINT, true)
        } else {
            return;
        };
        let (source, in_hint) = matched;

        let before = entity.confidence;
        let after = before.saturating_add(rule.boost.get());
        if after == before {
            return;
        }
        entity.confidence = after;

        // Record the rule's first keyword as the representative match;
        // the matcher reports "any keyword fired", not which one.
        let keyword = rule.keywords.first().cloned().unwrap_or_default();
        entity.provenance.record(
            Event::refinement(source, before, after, keyword, in_hint).with_reason(format!(
                "context keyword near `{}` (+{:.3})",
                entity.label.as_str(),
                rule.boost.get(),
            )),
        );
    }
}
