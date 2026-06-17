//! [`BoostRule`]: per-label keyword-boost rule.
//!
//! One rule per [`LabelRef`] declares the keyword set that
//! lifts confidence when one of those keywords appears within
//! `prefix_words` words before or `suffix_words` words after an
//! entity carrying that label. The window radii and the additive
//! `boost` are resolved at rule construction time — there are no
//! per-source overrides at apply time.
//!
//! Producers (the pattern crate today, future NER/LLM/custom
//! recognizer authors) hand the engine a `Vec<BoostRule>` keyed by
//! label. When several rules contribute to the same label (e.g.
//! two different SSN detectors both contributing to
//! `GOVERNMENT_ID`), the engine merges them by union of keywords —
//! see [`BoostRule::merge`].
//!
//! [`LabelRef`]: veil_core::entity::LabelRef

use std::collections::HashSet;

use hipstr::HipStr;
use veil_core::entity::LabelRef;
use veil_core::primitive::{Confidence, LanguageTag};

/// Default window radius in words *before* an entity match.
pub const DEFAULT_PREFIX_WORDS: usize = 5;

/// Default window radius in words *after* an entity match. Set
/// equal to [`DEFAULT_PREFIX_WORDS`] so trailing context like
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
    /// [`label`]: veil_core::entity::Entity::label
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
    /// `RecognizerInput.artifacts` when present, or via Unicode
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
}

impl BoostRule {
    /// Construct a rule for `label` with explicit window radii
    /// and `boost`. The rule is language-agnostic; use
    /// [`with_language`] to scope it. Most callers want
    /// [`BoostRule::for_label`] instead — it bakes in the default
    /// window / boost values.
    ///
    /// [`with_language`]: Self::with_language
    #[must_use]
    pub fn new(
        label: LabelRef,
        keywords: impl IntoIterator<Item = impl Into<HipStr<'static>>>,
        prefix_words: usize,
        suffix_words: usize,
        boost: Confidence,
    ) -> Self {
        Self {
            label,
            language: None,
            keywords: keywords.into_iter().map(Into::into).collect(),
            prefix_words,
            suffix_words,
            boost,
        }
    }

    /// Construct a rule for `label` using the crate's default
    /// [`prefix_words`], [`suffix_words`], and [`boost`]
    /// constants. The common case — recognizers building their
    /// own boost rules from declared keywords don't need to
    /// think about tuning knobs.
    ///
    /// [`prefix_words`]: DEFAULT_PREFIX_WORDS
    /// [`suffix_words`]: DEFAULT_SUFFIX_WORDS
    /// [`boost`]: DEFAULT_BOOST
    #[must_use]
    pub fn for_label(
        label: LabelRef,
        keywords: impl IntoIterator<Item = impl Into<HipStr<'static>>>,
    ) -> Self {
        Self::new(
            label,
            keywords,
            DEFAULT_PREFIX_WORDS,
            DEFAULT_SUFFIX_WORDS,
            Confidence::clamped(DEFAULT_BOOST),
        )
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
    pub fn applies_to_language(&self, hint: Option<&LanguageTag>) -> bool {
        match (&self.language, hint) {
            (None, _) => true,
            (Some(_), None) => true,
            (Some(scope), Some(hint)) => scope.matches(hint),
        }
    }

    /// Merge `other` into this rule by extending the keyword set
    /// with any keywords not already present. Window radii and
    /// `boost` are kept from `self` — callers that need different
    /// values per source should construct independent rules and
    /// keep them separate.
    ///
    /// # Panics
    ///
    /// Debug-asserts when the labels or languages differ. Merging
    /// across keys is a caller bug — rules are keyed by
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
