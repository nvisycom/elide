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

/// Per-label boost rule the [`Enhancer`] applies at runtime.
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
    /// `RecognizerContext.artifacts` when present, or via Unicode
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
        }
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
    }
}
