//! [`BoostRule`]: per-label keyword-boost rule.
//!
//! One rule per [`LabelRef`] declares the keyword set that
//! lifts confidence when one of those keywords appears within
//! `prefix_words` words before or `suffix_words` words after an
//! entity carrying that label. The window radii and the additive
//! `boost` are resolved at rule construction time; there are no
//! per-source overrides at apply time.
//!
//! Producers (the pattern crate today, future NER/LLM/custom
//! recognizer authors) hand the engine a `Vec<BoostRule>` keyed by
//! label. When several rules contribute to the same label (e.g.
//! two different SSN detectors both contributing to
//! `GOVERNMENT_ID`), the engine merges them by union of keywords;
//! see [`BoostRule::merge`].
//!
//! [`LabelRef`]: elide_core::entity::LabelRef

use std::collections::HashSet;

use elide_core::entity::LabelRef;
use elide_core::primitive::{Confidence, LanguageTag};
use hipstr::HipStr;

/// Default window radius in words *before* an entity match.
pub const DEFAULT_PREFIX_WORDS: usize = 5;

/// Default window radius in words *after* an entity match.
///
/// Set equal to [`DEFAULT_PREFIX_WORDS`] so trailing context like
/// "123-45-6789 (social security)" boosts the same as leading
/// context. Asymmetric windows surprise operators who rarely
/// realize the asymmetry exists, so we pick symmetric defaults.
pub const DEFAULT_SUFFIX_WORDS: usize = 5;

/// Default additive boost applied when a keyword fires.
pub const DEFAULT_BOOST: f32 = 0.35;

/// An entity-proximity condition: boost the rule's entity when an entity of
/// [`label`](Anchor::label) at or above [`min_confidence`](Anchor::min_confidence)
/// sits within the rule's window.
///
/// Where a keyword answers "is a currency *word* nearby", an anchor answers "is
/// a currency *entity* nearby" — the stronger signal for a value whose meaning
/// depends on an adjacent entity, such as a number that is a monetary amount
/// only beside an accepted currency. The confidence floor keeps a weak,
/// unconfirmed neighbour (an ambiguous currency token that never cleared its own
/// context gate) from acting as an anchor.
#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    /// The neighbouring entity's label (e.g. `currency`).
    pub label: LabelRef,
    /// The least confidence the neighbour must carry to anchor a boost.
    pub min_confidence: Confidence,
    /// When set, restrict the boost to entities whose own matched text is one of
    /// these: a dict-sourced anchor applies only to that dictionary's terms, so
    /// a `currency` entity from the currency-codes dictionary is number-anchored
    /// while one from another currency dictionary (an ambiguous crypto name) is
    /// not, even though both carry the same `currency` label. `None` applies to
    /// every entity of the rule's label; `Some` with an empty set matches no
    /// entity, so the rule never fires.
    pub self_terms: Option<HashSet<HipStr<'static>>>,
}

/// Per-label boost rule the [`Enhancer`] applies at runtime.
///
/// A rule boosts its entity from either (or both) of two signals: a *keyword*
/// in the surrounding word window, or an *[`anchor`](BoostRule::anchor)* — a
/// qualifying entity of another label in that window.
///
/// [`Enhancer`]: super::Enhancer
#[derive(Debug, Clone, PartialEq)]
pub struct BoostRule {
    /// Entity label this rule applies to. Each emitted
    /// `Entity<Text>` whose [`label`] matches is checked against
    /// this rule's keywords.
    ///
    /// [`label`]: elide_core::entity::Entity::label
    pub label: LabelRef,
    /// Language scope. `None` means the rule applies regardless
    /// of the per-call language hint; `Some(lang)` means the rule
    /// only fires when the caller's language matches, or when no
    /// hint is set (permissive fallback).
    pub language: Option<LanguageTag>,
    /// Keywords whose presence near a match lifts the entity's
    /// confidence. Stored as [`HipStr`] for cheap clones across
    /// per-pass rule sets.
    pub keywords: Vec<HipStr<'static>>,
    /// Window radius in words *before* the entity's match.
    /// Counted against the token artifact on
    /// `Context.artifacts` when present, or via Unicode
    /// word segmentation of the source text otherwise.
    pub prefix_words: usize,
    /// Window radius in words *after* the entity's match. Same
    /// source as [`prefix_words`].
    ///
    /// [`prefix_words`]: Self::prefix_words
    pub suffix_words: usize,
    /// Additive boost applied to the entity's confidence when a
    /// keyword fires. Clamped at the [`Confidence`] ceiling on
    /// apply.
    pub boost: Confidence,
    /// Whether a keyword must match on word boundaries. With the default
    /// `true`, the keyword `"AUD"` matches the token `AUD` but not the
    /// `aud` inside `audit`; set `false` for permissive substring matching
    /// (a keyword firing inside a longer word, e.g. `ssn` in `yourSSN`).
    pub word_boundary: bool,
    /// Optional entity-proximity condition. When set, a qualifying neighbour
    /// entity in the window boosts this rule's entity, in addition to any
    /// keyword match. A rule may carry an anchor with no keywords (boost purely
    /// on a neighbour) or both.
    pub anchor: Option<Anchor>,
}

impl BoostRule {
    /// Construct a rule for `label` with the crate's default window radii
    /// ([`DEFAULT_PREFIX_WORDS`] / [`DEFAULT_SUFFIX_WORDS`]), default
    /// [`boost`](DEFAULT_BOOST), and word-boundary matching. The rule is
    /// language-agnostic; override any knob with the `with_*` setters
    /// ([`with_window`], [`with_boost`], [`with_language`],
    /// [`with_word_boundary`]).
    ///
    /// [`with_window`]: Self::with_window
    /// [`with_boost`]: Self::with_boost
    /// [`with_language`]: Self::with_language
    /// [`with_word_boundary`]: Self::with_word_boundary
    #[must_use]
    pub fn new(
        label: LabelRef,
        keywords: impl IntoIterator<Item = impl Into<HipStr<'static>>>,
    ) -> Self {
        Self {
            label,
            language: None,
            keywords: keywords.into_iter().map(Into::into).collect(),
            prefix_words: DEFAULT_PREFIX_WORDS,
            suffix_words: DEFAULT_SUFFIX_WORDS,
            boost: Confidence::clamped(DEFAULT_BOOST),
            word_boundary: true,
            anchor: None,
        }
    }

    /// Boost this rule's entity when an entity of `label` at or above
    /// `min_confidence` sits within the window. Use with no keywords for a
    /// purely entity-anchored rule (e.g. a monetary amount beside a currency).
    #[must_use]
    pub fn with_anchor(mut self, label: LabelRef, min_confidence: Confidence) -> Self {
        self.anchor = Some(Anchor {
            label,
            min_confidence,
            self_terms: None,
        });
        self
    }

    /// Restrict an [anchored](Self::with_anchor) rule to entities whose own
    /// matched text is one of `terms`. A no-op without a prior anchor.
    #[must_use]
    pub fn with_self_terms(
        mut self,
        terms: impl IntoIterator<Item = impl Into<HipStr<'static>>>,
    ) -> Self {
        if let Some(anchor) = self.anchor.as_mut() {
            anchor.self_terms = Some(terms.into_iter().map(Into::into).collect());
        }
        self
    }

    /// Set the window radii (words before / after the match) the keywords are
    /// searched within.
    #[must_use]
    pub fn with_window(mut self, prefix_words: usize, suffix_words: usize) -> Self {
        self.prefix_words = prefix_words;
        self.suffix_words = suffix_words;
        self
    }

    /// Set the additive boost applied when a keyword fires.
    #[must_use]
    pub fn with_boost(mut self, boost: Confidence) -> Self {
        self.boost = boost;
        self
    }

    /// Scope this rule to a single language.
    ///
    /// At apply time the rule fires only when the caller's
    /// language hint matches `language`, or when no hint is set
    /// (permissive fallback).
    #[must_use]
    pub fn with_language(mut self, language: LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Set whether keywords match on word boundaries (`true`, the default) or
    /// as permissive substrings (`false`).
    #[must_use]
    pub fn with_word_boundary(mut self, word_boundary: bool) -> Self {
        self.word_boundary = word_boundary;
        self
    }

    /// Return `true` when this rule applies under the per-call
    /// language hint.
    ///
    /// - Language-agnostic rules (`self.language == None`)
    ///   always apply.
    /// - Language-scoped rules apply when the hint shares a
    ///   primary subtag with the scope (so a rule scoped to
    ///   `"en"` fires for `"en-US"` and `"en-GB"` hints), or
    ///   when no hint is set (permissive fallback so callers
    ///   who don't pass a language still get boosts).
    #[must_use]
    pub fn applies_to_language(&self, hints: &[&LanguageTag]) -> bool {
        match &self.language {
            // A language-agnostic rule always applies.
            None => true,
            // A scoped rule applies when the call asserts no language (the
            // permissive fallback), or when *any* asserted/detected language
            // matches the rule's scope, so a multilingual call activates every
            // one of its languages' per-language context.
            Some(scope) => hints.is_empty() || hints.iter().any(|hint| scope.matches(hint)),
        }
    }

    /// Merge `other` into this rule by extending the keyword set
    /// with any keywords not already present. Window radii and
    /// `boost` are kept from `self`; callers that need different
    /// values per source should construct independent rules and
    /// keep them separate.
    ///
    /// # Panics
    ///
    /// Debug-asserts when the labels or languages differ. Merging
    /// across keys is a caller bug: rules are keyed by
    /// `(label, language)` and the engine looks them up by both.
    pub fn merge(&mut self, other: BoostRule) {
        debug_assert_eq!(
            self.label, other.label,
            "BoostRule::merge requires matching labels",
        );
        debug_assert_eq!(
            self.language, other.language,
            "BoostRule::merge requires matching languages",
        );
        let existing: HashSet<&str> = self.keywords.iter().map(HipStr::as_str).collect();
        let additions: Vec<HipStr<'static>> = other
            .keywords
            .into_iter()
            .filter(|kw| !existing.contains(kw.as_str()))
            .collect();
        self.keywords.extend(additions);
        // Keep an anchor from either side; `self`'s wins when both carry one.
        if self.anchor.is_none() {
            self.anchor = other.anchor;
        }
    }
}
