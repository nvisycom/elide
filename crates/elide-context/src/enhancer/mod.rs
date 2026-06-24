//! [`Enhancer`]: post-recognition keyword-boost pass for any
//! [`Entity<Text>`] regardless of which recognizer produced it.

use std::collections::HashMap;
use std::ops::Range;

use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::TextRecognizable;
use elide_core::primitive::Confidence;
use hipstr::HipStr;

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

    /// Apply boost rules to `entities` in place, lifting confidence where a
    /// keyword fires, and return one [`Boost`] per lift for the caller to
    /// record in provenance.
    ///
    /// For each entity: walk every rule registered for its label whose
    /// language scope applies under `ctx.language`, check a window of
    /// `prefix_words`/`suffix_words` around the entity's
    /// [`recognized_range`] (and the out-of-band hints), and on a hit lift
    /// confidence by the rule's `boost` (saturating at the [`Confidence`]
    /// ceiling). The in-text and hint paths are independent — at most one
    /// boost per rule fires per entity (window first, hint as fallback) — so
    /// a long keyword list can't double-dip.
    ///
    /// Reads only the modality-free fields (`label`, `recognized_range`,
    /// `confidence`), so it works for any modality. It does *not* build the
    /// provenance event: it returns each [`Boost`] with the matched hint's
    /// *index* (when the match came from a hint), for the caller to record.
    ///
    /// [`Confidence`]: elide_core::primitive::Confidence
    /// [`recognized_range`]: elide_core::entity::Entity::recognized_range
    pub fn enhance<M: TextRecognizable>(
        &self,
        entities: &mut [Entity<M>],
        ctx: &Context<'_>,
    ) -> Vec<Boost> {
        let mut boosts = Vec::new();
        if self.rules.is_empty() {
            return boosts;
        }
        for (entity_index, entity) in entities.iter_mut().enumerate() {
            self.enhance_one(entity_index, entity, ctx, &mut boosts);
        }
        boosts
    }

    fn enhance_one<M: TextRecognizable>(
        &self,
        entity_index: usize,
        entity: &mut Entity<M>,
        ctx: &Context<'_>,
        boosts: &mut Vec<Boost>,
    ) {
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
            if let Some(boost) = self.apply_rule(entity_index, entity, rule, ctx) {
                boosts.push(boost);
            }
        }
    }

    fn apply_rule<M: TextRecognizable>(
        &self,
        entity_index: usize,
        entity: &mut Entity<M>,
        rule: &BoostRule,
        ctx: &Context<'_>,
    ) -> Option<Boost> {
        // The in-text window path needs the entity's byte range into the
        // recognized text; the out-of-band hint path does not. An entity with
        // no `recognized_range` (e.g. a natively-located VLM box) can still be
        // boosted by a hint, just not by its surrounding word window.
        let window = entity
            .recognized_range
            .clone()
            .and_then(|range| self.window_match(&range, rule, ctx));

        // Window first; the hint path reports *which* hint fired so the
        // caller can record its location. The in-text path additionally
        // carries the keyword's *stream* range so the caller can resolve a
        // native location for it, symmetric with the hint's own location.
        let (source, hint_index, keyword_range) = if let Some(keyword_range) = window {
            (EVENT_SOURCE_WINDOW, None, Some(keyword_range))
        } else if let Some(i) = ctx
            .hints
            .iter()
            .position(|h| self.matcher.any_match(h, &[], &rule.keywords).is_some())
        {
            (EVENT_SOURCE_HINT, Some(i), None)
        } else {
            return None;
        };

        let before = entity.confidence;
        let after = before.saturating_add(rule.boost.get());
        if after == before {
            return None;
        }
        entity.confidence = after;

        // The rule's first keyword stands in for the match — the matcher
        // reports "any keyword fired", not which one.
        let keyword = rule.keywords.first().cloned().unwrap_or_default();
        Some(Boost {
            entity_index,
            source,
            before,
            after,
            keyword,
            hint_index,
            keyword_range,
            amount: rule.boost.get(),
        })
    }

    /// Match the rule's keywords in the word/token window around `range`,
    /// returning the matched keyword's *stream* byte range (rebased into the
    /// recognized text) when one fires.
    fn window_match(
        &self,
        range: &Range<usize>,
        rule: &BoostRule,
        ctx: &Context<'_>,
    ) -> Option<Range<usize>> {
        // Prefer the token stream when the producer reached this entity. Fall
        // back to the word-segmented substring window whenever the token slice
        // would be empty; that covers `tokens: None`, `tokens: Some(&[])`, and
        // the "tokens present but none overlap the entity" case (e.g. an NLP
        // engine that only tokenized part of the document).
        let token_slice = ctx
            .tokens
            .map(|toks| {
                slice_tokens_around(toks, range.clone(), rule.prefix_words, rule.suffix_words)
            })
            .unwrap_or(&[]);
        // `window_offset` is the window's stream-byte start: a match the
        // matcher reports is window-relative, so adding this rebases it into
        // stream coordinates for `M::locate`.
        let (snippet, tokens_in_window, window_offset): (&str, &[Token], usize) =
            if token_slice.is_empty() {
                let (snippet, offset) = word_window(
                    ctx.text,
                    range.clone(),
                    rule.prefix_words,
                    rule.suffix_words,
                );
                (snippet, &[], offset)
            } else {
                let (snippet, offset) = token_span(ctx.text, token_slice, range.clone());
                (snippet, token_slice, offset)
            };
        let m = self
            .matcher
            .any_match(snippet, tokens_in_window, &rule.keywords)?;
        Some(window_offset + m.start..window_offset + m.end)
    }
}

/// One confidence lift the enhancer applied, for the caller to record in
/// provenance.
///
/// Modality-free: the matched hint is referenced by *index* into the
/// context hints, so the caller (which holds the located hints) can attach
/// the hint's location.
#[derive(Debug, Clone)]
pub struct Boost {
    /// Index of the entity that was lifted, into the slice passed to
    /// [`Enhancer::enhance`].
    pub entity_index: usize,
    /// Event source tag (`"context"` for a window match, `"context-hint"`
    /// for a hint match).
    pub source: &'static str,
    /// Confidence before the lift.
    pub before: Confidence,
    /// Confidence after the lift.
    pub after: Confidence,
    /// Representative keyword (the rule's first).
    pub keyword: HipStr<'static>,
    /// Index of the matched hint into the context hints, or `None` for an
    /// in-text-window match.
    pub hint_index: Option<usize>,
    /// For an in-text-window match, the **stream** byte range of the keyword
    /// that fired (into the recognized-text stream), so the caller can
    /// resolve it to a native location via [`locate`] — symmetric with the
    /// located hint a `hint_index` match carries. `None` for a hint match
    /// (the location lives on the hint).
    ///
    /// [`locate`]: elide_core::modality::TextRecognizable::locate
    pub keyword_range: Option<Range<usize>>,
    /// The boost amount applied, as a bare `f32` (for the reason string).
    pub amount: f32,
}
