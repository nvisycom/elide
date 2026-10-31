//! [`Pattern`] rule and its [`Variant`]s.

mod compiled;

use derive_builder::Builder;
use elide_core::entity::LabelRef;
use elide_core::primitive::{Confidence, CountryCode, LanguageTag};
use elide_core::{Error, ErrorKind, Result};
use regex::Regex;
use serde::Deserialize;

pub(crate) use self::compiled::CompiledPattern;
use crate::context::RuleContext;

/// One regex strategy inside a [`Pattern`] rule.
///
/// A variant pairs a regex source with the confidence stamped on
/// every match it produces and, optionally, a validator name
/// resolved against the [`ValidatorRegistry`] at recognizer-build
/// time so structurally-suspect matches can be dropped.
///
/// # Spans
///
/// A match's span is the whole match, unless the regex defines a capture group
/// named [`ENTITY_GROUP`] (`entity`): then that group is the span, and the rest
/// of the match is context that must be present but is not part of the entity.
/// This stands in for lookaround, which the `regex` crate does not support:
///
/// ```toml
/// # A trailing boundary: `2001:db8::1` matches, the prefix of `2001:db8::1g` does not.
/// regex = '(?P<entity>[0-9A-Fa-f:]+)(?:$|[^0-9A-Za-z:])'
/// ```
///
/// Context outside the group is consumed, so the next match starts after it.
/// A match whose `entity` group did not participate, or matched nothing, is
/// skipped, and a validator sees only the group's text.
///
/// [`ValidatorRegistry`]: crate::validators::ValidatorRegistry
/// [`ENTITY_GROUP`]: Self::ENTITY_GROUP
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Variant {
    /// Regex source. Compiled to a [`::regex::Regex`] by
    /// [`PatternRecognizer::build`]. An [`ENTITY_GROUP`] capture group, when
    /// present, narrows each match to that group (see [Spans](Self#spans)).
    ///
    /// [`PatternRecognizer::build`]: crate::PatternRecognizer
    /// [`ENTITY_GROUP`]: Self::ENTITY_GROUP
    pub regex: String,
    /// Confidence stamped on every match, before any
    /// post-recognition keyword boost.
    #[serde(default = "default_score")]
    pub score: Confidence,
    /// Validator name resolved against the [`ValidatorRegistry`].
    /// Matches that fail validation are dropped.
    ///
    /// [`ValidatorRegistry`]: crate::validators::ValidatorRegistry
    #[serde(default)]
    pub validator: Option<String>,
}

impl Variant {
    /// The capture-group name that marks a match's entity span: a regex with a
    /// `(?P<entity>…)` group reports that group instead of the whole match.
    pub const ENTITY_GROUP: &'static str = "entity";

    /// Construct a variant from a regex source.
    ///
    /// `score` defaults to [`Confidence::MAX`] and `validator` to
    /// `None`; override with [`with_score`] / [`with_validator`].
    ///
    /// # Errors
    ///
    /// Returns a validation error when `regex` is not a valid
    /// regular expression.
    ///
    /// [`with_score`]: Self::with_score
    /// [`with_validator`]: Self::with_validator
    pub fn new(regex: impl Into<String>) -> Result<Self> {
        let regex = regex.into();
        if let Err(e) = Regex::new(&regex) {
            return Err(Error::new(
                ErrorKind::Configuration,
                format!("invalid regex: {e}"),
            ));
        }
        Ok(Self {
            regex,
            score: Confidence::MAX,
            validator: None,
        })
    }

    /// Set the per-match confidence score.
    #[must_use]
    pub fn with_score(mut self, score: Confidence) -> Self {
        self.score = score;
        self
    }

    /// Set the validator name to run on every match.
    ///
    /// The name is resolved against the [`ValidatorRegistry`] when
    /// the parent [`PatternRecognizer`] is built; unknown names
    /// surface as a build-time error.
    ///
    /// [`ValidatorRegistry`]: crate::validators::ValidatorRegistry
    /// [`PatternRecognizer`]: crate::PatternRecognizer
    #[must_use]
    pub fn with_validator(mut self, name: impl Into<String>) -> Self {
        self.validator = Some(name.into());
        self
    }
}

fn default_score() -> Confidence {
    Confidence::MAX
}

/// Pattern detection rule: one or more candidate labels, optional keyword
/// boosts, and one or more [`Variant`]s.
///
/// A rule groups several regex strategies under a shared entity-type
/// candidate list plus a shared context-keyword list. Every variant emits
/// the same [label](Pattern::labels) and shares the rule's
/// [`context`](Pattern::context).
///
/// # Candidate labels
///
/// [`labels`](Pattern::labels) is an ordered list, most-specific first. At
/// recognize time the rule emits the **first** candidate the request's
/// [catalog](elide_core::recognition::Context::catalog) declares, so
/// one pattern can serve consumers that opted into a fine-grained label and
/// those that only enabled a coarser one. A rule that emits a single label
/// carries a one-element list.
///
/// # Examples
///
/// ```
/// use elide_core::entity::builtins;
/// use elide_core::primitive::Confidence;
/// use elide_pattern::{Pattern, Variant};
///
/// let variant = Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
///     .expect("ssn variant builds")
///     .with_score(Confidence::clamped(0.9))
///     .with_validator("ssn");
///
/// let ssn = Pattern::builder()
///     .with_name("ssn")
///     .with_labels(vec![builtins::GOVERNMENT_ID.to_ref()])
///     .with_context(vec!["ssn".to_owned(), "social security".to_owned()])
///     .with_variants(vec![variant])
///     .build()
///     .expect("ssn rule builds");
/// ```
///
/// [`PatternRecognizer`]: crate::PatternRecognizer
#[derive(Debug, Clone, PartialEq, Builder, Deserialize)]
#[builder(
    name = "PatternBuilder",
    pattern = "owned",
    setter(into, strip_option, prefix = "with"),
    build_fn(error = "Error")
)]
pub struct Pattern {
    /// Human-readable identifier surfaced in trail provenance (e.g.
    /// `"ssn"`, `"credit_card"`).
    pub name: String,
    /// Candidate entity labels, most-specific first. The recognizer emits the
    /// first candidate the request catalog declares. In TOML: `labels =
    /// ["specific", "general"]` (a single-candidate rule is `labels = ["x"]`).
    pub labels: Vec<LabelRef>,
    /// Evidence near a match that raises its confidence: keywords, in any
    /// language or per language, and an anchor label.
    #[builder(default)]
    #[serde(default)]
    pub context: RuleContext,
    /// Pattern variants. At least one is required to produce matches;
    /// the recognizer skips rules with an empty variant list.
    pub variants: Vec<Variant>,
    /// BCP-47 language tags the rule applies to. Empty means "any
    /// language"; otherwise the recognizer skips the rule when the
    /// per-call language hint is not in the list.
    #[builder(default)]
    #[serde(default)]
    pub languages: Vec<LanguageTag>,
    /// ISO 3166-1 alpha-2 country codes the rule applies to.
    /// Empty means "any country", the rule fires regardless of
    /// the per-call jurisdiction hint. Use this to scope a
    /// pattern to specific national formats (e.g. `["US"]` for
    /// the SSN regex).
    #[builder(default)]
    #[serde(default)]
    pub countries: Vec<CountryCode>,
}

impl Pattern {
    /// Start a chainable builder.
    ///
    /// Required fields: `name`, `labels`, `variants`.
    #[must_use]
    pub fn builder() -> PatternBuilder {
        PatternBuilder::default()
    }

    /// Parse a rule from a TOML source.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the TOML is malformed or
    /// missing required fields.
    pub fn from_toml(raw: &str) -> Result<Self> {
        toml::from_str(raw)
            .map_err(|e| Error::new(ErrorKind::Configuration, format!("pattern rule TOML: {e}")))
    }
}
