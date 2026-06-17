//! [`Enhancer`]: post-recognition keyword-boost pass for any
//! [`Entity<Text>`] regardless of which recognizer produced it.

use std::collections::HashMap;

use nvisy_core::entity::{Entity, EntityLabelRef, TrailStep};
use nvisy_core::modality::Text;

use crate::io::Token;
use crate::matching::KeywordMatcher;
use crate::rule::BoostRule;

mod context;
mod window;

pub use self::context::Context;
use self::window::{slice_tokens_around, token_span, word_window};

/// Source name stamped onto refinement [`TrailStep`]s the
/// enhancer appends when the in-text word window fires.
const TRAIL_SOURCE_WINDOW: &str = "context";

/// Source name stamped onto refinement [`TrailStep`]s the
/// enhancer appends when an out-of-band hint fires.
const TRAIL_SOURCE_HINT: &str = "context-hint";

/// Post-recognition enhancer. Holds a label-keyed [`BoostRule`]
/// map plus the keyword-matching strategy, and lifts the
/// confidence of each text entity whose label has a rule and
/// whose surrounding word window contains one of the rule's
/// keywords.
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
/// [`SubstringMatcher`]: crate::SubstringMatcher
/// [`LemmaMatcher`]: crate::LemmaMatcher
pub struct Enhancer {
    /// Rules bucketed by label. Within one bucket, each entry is
    /// a distinct `(language)` scope; rules sharing the same
    /// `(label, language)` are pre-merged via [`BoostRule::merge`]
    /// at construction. Per-entity application looks up the
    /// bucket once by label, then walks the small inner vec
    /// filtering on the per-call language hint.
    rules: HashMap<EntityLabelRef, Vec<BoostRule>>,
    matcher: Box<dyn KeywordMatcher>,
}

impl Enhancer {
    /// Construct from a rule iterator and matcher. Rules sharing
    /// the same `(label, language)` are merged via
    /// [`BoostRule::merge`]; rules with the same label but
    /// distinct languages live as separate entries inside the
    /// label's bucket.
    pub fn new(
        rules: impl IntoIterator<Item = BoostRule>,
        matcher: Box<dyn KeywordMatcher>,
    ) -> Self {
        let mut buckets: HashMap<EntityLabelRef, Vec<BoostRule>> = HashMap::new();
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
    /// (saturating at the [`Confidence`] ceiling) plus append a
    /// [`Refinement`] trail step.
    ///
    /// The in-text and hint paths are independent — at most one
    /// boost per rule fires per entity (window first, hint as
    /// fallback) so a rule with a long keyword list can't
    /// double-dip.
    ///
    /// [`Confidence`]: nvisy_core::primitive::Confidence
    /// [`Refinement`]: nvisy_core::entity::TrailStepKind::Refinement
    pub fn enhance(&self, entities: &mut [Entity<Text>], ctx: &Context<'_>) {
        if self.rules.is_empty() {
            return;
        }
        for entity in entities {
            self.enhance_one(entity, ctx);
        }
    }

    fn enhance_one(&self, entity: &mut Entity<Text>, ctx: &Context<'_>) {
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

    fn apply_rule(&self, entity: &mut Entity<Text>, rule: &BoostRule, ctx: &Context<'_>) {
        let start = entity.location.start;
        let end = entity.location.end;

        // Prefer the token stream when the producer reached this
        // entity. Fall back to the word-segmented substring window
        // whenever the token slice would be empty — that covers
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

        let source = if self
            .matcher
            .any_match(snippet, tokens_in_window, &rule.keywords)
        {
            TRAIL_SOURCE_WINDOW
        } else if ctx
            .hints
            .iter()
            .any(|h| self.matcher.any_match(h, &[], &rule.keywords))
        {
            TRAIL_SOURCE_HINT
        } else {
            return;
        };

        let original = entity.confidence;
        let adjusted = original.saturating_add(rule.boost.get());
        if adjusted == original {
            return;
        }
        entity.confidence = adjusted;

        entity.trail.push(TrailStep::refinement(
            source,
            original,
            adjusted,
            format!(
                "context keyword near `{}` (+{:.3})",
                entity.label.as_str(),
                rule.boost.get(),
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use nvisy_core::entity::{
        EntityLabelRef, PatternProvenance, TrailProvenance, TrailStepKind, builtins,
    };
    use nvisy_core::modality::{Text, TextLocation};
    use nvisy_core::primitive::Confidence;

    use super::*;
    use crate::SubstringMatcher;

    fn govid_label() -> EntityLabelRef {
        builtins::GOVERNMENT_ID.label_ref()
    }

    fn person_label() -> EntityLabelRef {
        builtins::PERSON_NAME.label_ref()
    }

    fn entity(label: EntityLabelRef, start: usize, end: usize, score: f64) -> Entity<Text> {
        let confidence = Confidence::new(score).unwrap();
        let step = TrailStep::recognition(
            "test",
            confidence,
            TrailProvenance::Pattern(PatternProvenance::DenyList),
            "test fixture",
        );
        Entity::builder()
            .with_label(label)
            .with_trail(vec![step])
            .with_confidence(confidence)
            .with_location(TextLocation::new(start, end))
            .build()
            .expect("entity builds")
    }

    fn enhancer(rules: Vec<BoostRule>) -> Enhancer {
        Enhancer::new(rules, Box::new(SubstringMatcher))
    }

    fn rule(
        label: EntityLabelRef,
        keywords: &[&'static str],
        prefix: usize,
        suffix: usize,
        boost: f64,
    ) -> BoostRule {
        BoostRule::new(
            label,
            keywords.iter().copied(),
            prefix,
            suffix,
            Confidence::clamped(boost),
        )
    }

    #[test]
    fn boosts_entity_when_keyword_in_word_window() {
        let enhancer = enhancer(vec![rule(
            govid_label(),
            &["ssn", "social security"],
            5,
            5,
            0.2,
        )]);
        let text = "Your SSN: 123-45-6789";
        let mut entities = vec![entity(govid_label(), 10, 21, 0.6)];
        enhancer.enhance(&mut entities, &Context::new(text));
        assert!(entities[0].confidence.get() > 0.6);
        assert!(
            entities[0]
                .trail
                .iter()
                .any(|s| matches!(s.kind, TrailStepKind::Refinement)),
        );
    }

    #[test]
    fn suffix_zero_ignores_trailing_keyword() {
        // Prefix-only: trailing keyword must not boost.
        let enhancer = enhancer(vec![rule(govid_label(), &["social"], 5, 0, 0.2)]);
        let text = "123-45-6789 (social security number)";
        let mut entities = vec![entity(govid_label(), 0, 11, 0.6)];
        let before = entities[0].confidence.get();
        enhancer.enhance(&mut entities, &Context::new(text));
        assert_eq!(entities[0].confidence.get(), before);
    }

    #[test]
    fn skips_entity_with_no_rule_for_label() {
        let enhancer = enhancer(vec![rule(govid_label(), &["ssn"], 5, 5, 0.2)]);
        let text = "Mr. Smith is named in the report.";
        let mut entities = vec![entity(person_label(), 4, 9, 0.5)];
        let before = entities[0].confidence.get();
        enhancer.enhance(&mut entities, &Context::new(text));
        assert_eq!(entities[0].confidence.get(), before);
    }

    #[test]
    fn window_bounds_the_search() {
        // 2-word prefix / 2-word suffix: "far_keyword" is at the
        // start; the entity is after many filler words.
        let enhancer = enhancer(vec![rule(govid_label(), &["far_keyword"], 2, 2, 0.2)]);
        let text = "far_keyword here is some filler between the keyword and XYZ here";
        let xyz_start = text.find("XYZ").unwrap();
        let xyz_end = xyz_start + "XYZ".len();
        let mut entities = vec![entity(govid_label(), xyz_start, xyz_end, 0.6)];
        let before = entities[0].confidence.get();
        enhancer.enhance(&mut entities, &Context::new(text));
        assert_eq!(entities[0].confidence.get(), before);
    }

    #[test]
    fn boost_saturates_at_one() {
        let enhancer = enhancer(vec![rule(govid_label(), &["here"], 5, 5, 0.9)]);
        let text = "the value is right here in plain sight";
        let mut entities = vec![entity(govid_label(), 16, 21, 0.95)];
        enhancer.enhance(&mut entities, &Context::new(text));
        assert!((entities[0].confidence.get() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn duplicate_label_rules_merge_keywords() {
        // Two rules for the same label, each contributing a
        // distinct keyword. The merged rule must trigger boosts
        // for matches near keywords from *either* original source,
        // proving the keyword union survived the merge (not just
        // last-write-wins).
        let make_enhancer = || {
            enhancer(vec![
                rule(govid_label(), &["ssn"], 5, 5, 0.2),
                rule(govid_label(), &["tax id"], 5, 5, 0.2),
            ])
        };
        assert_eq!(make_enhancer().len(), 1);

        // Keyword only from the first rule.
        let ssn_only = "ssn: 123-45-6789";
        let ssn_entity_start = ssn_only.find("123").unwrap();
        let ssn_entity_end = ssn_entity_start + "123-45-6789".len();
        let mut from_first = vec![entity(govid_label(), ssn_entity_start, ssn_entity_end, 0.6)];
        make_enhancer().enhance(&mut from_first, &Context::new(ssn_only));
        assert!(
            from_first[0].confidence.get() > 0.6,
            "keyword `ssn` from the first rule must still boost after merge",
        );

        // Keyword only from the second rule.
        let taxid_only = "tax id: 987-65-4329";
        let tax_entity_start = taxid_only.find("987").unwrap();
        let tax_entity_end = tax_entity_start + "987-65-4329".len();
        let mut from_second = vec![entity(govid_label(), tax_entity_start, tax_entity_end, 0.6)];
        make_enhancer().enhance(&mut from_second, &Context::new(taxid_only));
        assert!(
            from_second[0].confidence.get() > 0.6,
            "keyword `tax id` from the second rule must still boost after merge",
        );
    }

    #[test]
    fn word_window_handles_unicode() {
        // 3-word prefix reaches "café" past "naïve" and "resume".
        let enhancer = enhancer(vec![rule(govid_label(), &["café"], 3, 0, 0.2)]);
        let text = "café naïve resume — 123-45-6789";
        let entity_start = text.find("123").unwrap();
        let entity_end = entity_start + "123-45-6789".len();
        let mut entities = vec![entity(govid_label(), entity_start, entity_end, 0.6)];
        enhancer.enhance(&mut entities, &Context::new(text));
        assert!(
            entities[0].confidence.get() > 0.6,
            "unicode word should be reachable within 3-word prefix",
        );
    }

    #[test]
    fn empty_tokens_slice_matches_none_behaviour() {
        // `Some(&[])` must not collapse the snippet to entity
        // bytes — it should fall back to the word-window path
        // just like `None`.
        let enhancer = enhancer(vec![rule(govid_label(), &["ssn"], 5, 5, 0.2)]);
        let text = "Your SSN: 123-45-6789";
        let mut from_none = vec![entity(govid_label(), 10, 21, 0.6)];
        let mut from_empty = vec![entity(govid_label(), 10, 21, 0.6)];
        enhancer.enhance(&mut from_none, &Context::new(text));
        enhancer.enhance(&mut from_empty, &Context::new(text).with_tokens(&[]));
        assert_eq!(
            from_none[0].confidence.get(),
            from_empty[0].confidence.get(),
            "Some(&[]) must behave identically to None",
        );
        assert!(
            from_empty[0].confidence.get() > 0.6,
            "empty tokens slice must still allow the word-window fallback to boost",
        );
    }

    #[test]
    fn token_path_counts_words_against_token_stream() {
        // 1-word prefix, 0-word suffix: the only word the
        // prefix reaches is the immediate predecessor token
        // "Your". The tokenizer here treats "social security"
        // as a single compound token outside the window, so the
        // keyword "social security" must NOT fire.
        let enhancer = enhancer(vec![rule(govid_label(), &["social security"], 1, 0, 0.2)]);
        let text = "social security: Your 123-45-6789";
        let entity_start = text.find("123").unwrap();
        let entity_end = entity_start + "123-45-6789".len();
        let tokens: Vec<Token> = vec![
            Token::from_text("social security", 0..15),
            Token::from_text("Your", 17..21),
            Token::from_text("123-45-6789", 22..33),
        ];
        let mut entities = vec![entity(govid_label(), entity_start, entity_end, 0.6)];
        let before = entities[0].confidence.get();
        enhancer.enhance(&mut entities, &Context::new(text).with_tokens(&tokens));
        assert_eq!(
            entities[0].confidence.get(),
            before,
            "1-word prefix should not reach the `social security` token two positions back",
        );
    }

    #[test]
    fn lemma_matcher_boosts_on_morphological_variant() {
        // Substring matcher would miss `running` for keyword
        // `run`. Lemma matcher reads the lemma directly off the
        // token and boosts.
        let enhancer = Enhancer::new(
            vec![rule(govid_label(), &["run"], 5, 5, 0.2)],
            Box::new(crate::LemmaMatcher),
        );
        let text = "They were running 123-45-6789 across the system";
        let entity_start = text.find("123").unwrap();
        let entity_end = entity_start + "123-45-6789".len();
        let tokens: Vec<Token> = vec![
            Token::from_text("They", 0..4),
            Token::from_text("were", 5..9),
            Token::from_text("running", 10..17).with_lemma("run"),
            Token::from_text("123-45-6789", 18..29),
            Token::from_text("across", 30..36),
            Token::from_text("the", 37..40),
            Token::from_text("system", 41..47),
        ];
        let mut entities = vec![entity(govid_label(), entity_start, entity_end, 0.6)];
        enhancer.enhance(&mut entities, &Context::new(text).with_tokens(&tokens));
        assert!(
            entities[0].confidence.get() > 0.6,
            "lemma matcher should match `run` against the `running` token's lemma",
        );
    }

    #[test]
    fn tokens_with_no_overlap_fall_back_to_word_window() {
        // Tokens cover the first half of the document; the entity
        // is in the second half, outside any token's range. The
        // word-window path must still reach the keyword.
        let enhancer = enhancer(vec![rule(govid_label(), &["ssn"], 5, 5, 0.2)]);
        let text = "First half of the document. Your SSN: 123-45-6789";
        let entity_start = text.find("123").unwrap();
        let entity_end = entity_start + "123-45-6789".len();
        let tokens: Vec<Token> = vec![
            Token::from_text("First", 0..5),
            Token::from_text("half", 6..10),
            Token::from_text("of", 11..13),
            Token::from_text("the", 14..17),
            Token::from_text("document", 18..26),
        ];
        let mut entities = vec![entity(govid_label(), entity_start, entity_end, 0.6)];
        enhancer.enhance(&mut entities, &Context::new(text).with_tokens(&tokens));
        assert!(
            entities[0].confidence.get() > 0.6,
            "tokens that don't overlap the entity must fall back to the word window",
        );
    }

    #[test]
    fn out_of_band_hint_boosts_when_window_is_empty() {
        // Cell-only text has no surrounding context — the word
        // window walk finds nothing — but the caller supplies the
        // CSV column header as an out-of-band hint that contains
        // a rule keyword. Confidence must lift, and the trail
        // step must mark the source as `context-hint`.
        let enhancer = enhancer(vec![rule(govid_label(), &["ssn"], 5, 5, 0.2)]);
        let text = "123-45-6789";
        let hints = ["ssn".to_owned()];
        let mut entities = vec![entity(govid_label(), 0, 11, 0.6)];
        enhancer.enhance(&mut entities, &Context::new(text).with_hints(&hints));
        assert!(
            entities[0].confidence.get() > 0.6,
            "out-of-band hint matching a rule keyword must boost",
        );
        assert!(
            entities[0].trail.iter().any(|s| s.source == "context-hint"),
            "trail step must record the hint-source provenance",
        );
    }

    #[test]
    fn hint_path_is_independent_of_window_path() {
        // The in-text window already fires, so the hint path
        // shouldn't double-boost. Exactly one refinement step
        // appears on the entity.
        let enhancer = enhancer(vec![rule(govid_label(), &["ssn"], 5, 5, 0.2)]);
        let text = "Your SSN: 123-45-6789";
        let hints = ["ssn".to_owned()];
        let mut entities = vec![entity(govid_label(), 10, 21, 0.6)];
        enhancer.enhance(&mut entities, &Context::new(text).with_hints(&hints));
        let refinements = entities[0]
            .trail
            .iter()
            .filter(|s| matches!(s.kind, TrailStepKind::Refinement))
            .count();
        assert_eq!(refinements, 1, "rule must boost at most once per entity");
    }
}
