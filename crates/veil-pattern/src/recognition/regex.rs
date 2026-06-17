//! [`Regex`] rule and its [`Variant`]s.

use derive_builder::Builder;
use nvisy_core::Error;
use nvisy_core::entity::EntityLabelRef;
use nvisy_core::primitive::{Confidence, CountryCode, LanguageTag};
use serde::Deserialize;

use super::context::Context;

/// One regex strategy inside a [`Regex`] rule.
///
/// A variant pairs a regex source with the confidence stamped on
/// every match it produces and, optionally, a validator name
/// resolved against the [`ValidatorRegistry`] at recognizer-build
/// time so structurally-suspect matches can be dropped.
///
/// [`ValidatorRegistry`]: crate::validators::ValidatorRegistry
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Variant {
    /// Regex source. Compiled to a [`::regex::Regex`] by
    /// [`PatternRecognizer::build`].
    ///
    /// [`PatternRecognizer::build`]: super::PatternRecognizer
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
    pub fn new(regex: impl Into<String>) -> Result<Self, Error> {
        let regex = regex.into();
        if let Err(e) = ::regex::Regex::new(&regex) {
            return Err(Error::validation(
                format!("invalid regex: {e}"),
                "nvisy-pattern",
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
    /// [`PatternRecognizer`]: super::PatternRecognizer
    #[must_use]
    pub fn with_validator(mut self, name: impl Into<String>) -> Self {
        self.validator = Some(name.into());
        self
    }
}

fn default_score() -> Confidence {
    Confidence::MAX
}

/// Regex detection rule: one label, optional keyword boosts, and
/// one or more [`Variant`]s.
///
/// A rule groups several regex strategies under a single entity
/// type plus a shared context-keyword list. Every variant emits
/// the same [`label`]; context keywords are harvested by
/// [`PatternRecognizer`] into a wrapping boost layer and are
/// never read by the rule itself.
///
/// # Examples
///
/// ```
/// use nvisy_core::entity::builtins;
/// use nvisy_core::primitive::Confidence;
/// use nvisy_pattern::{Regex, Variant};
///
/// let variant = Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
///     .expect("ssn variant builds")
///     .with_score(Confidence::clamped(0.9))
///     .with_validator("ssn");
///
/// let ssn = Regex::builder()
///     .with_name("ssn")
///     .with_label(builtins::GOVERNMENT_ID.label_ref())
///     .with_context(vec!["ssn".to_owned(), "social security".to_owned()])
///     .with_variants(vec![variant])
///     .build()
///     .expect("ssn rule builds");
/// ```
///
/// [`label`]: Regex::label
/// [`PatternRecognizer`]: super::PatternRecognizer
#[derive(Debug, Clone, PartialEq, Builder, Deserialize)]
#[builder(
    name = "RegexBuilder",
    pattern = "owned",
    setter(into, strip_option, prefix = "with"),
    build_fn(error = "Error")
)]
pub struct Regex {
    /// Human-readable identifier surfaced in trail provenance (e.g.
    /// `"ssn"`, `"credit_card"`).
    pub name: String,
    /// Entity label every variant emits.
    pub label: EntityLabelRef,
    /// Context keywords that lift confidence when one of them
    /// appears near a match. Either a flat list applied
    /// regardless of language, or a per-language map.
    #[builder(default)]
    #[serde(default)]
    pub context: Context,
    /// Regex variants. At least one is required to produce matches;
    /// the recognizer skips rules with an empty variant list.
    pub variants: Vec<Variant>,
    /// BCP-47 language tags the rule applies to. Empty means "any
    /// language"; otherwise the recognizer skips the rule when the
    /// per-call language hint is not in the list.
    #[builder(default)]
    #[serde(default)]
    pub languages: Vec<LanguageTag>,
    /// ISO 3166-1 alpha-2 country codes the rule applies to.
    /// Empty means "any country" — the rule fires regardless of
    /// the per-call jurisdiction hint. Use this to scope a
    /// pattern to specific national formats (e.g. `["US"]` for
    /// the SSN regex).
    #[builder(default)]
    #[serde(default)]
    pub countries: Vec<CountryCode>,
}

impl Regex {
    /// Start a chainable builder.
    ///
    /// Required fields: `name`, `label`, `variants`.
    #[must_use]
    pub fn builder() -> RegexBuilder {
        RegexBuilder::default()
    }

    /// Parse a rule from a TOML source.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the TOML is malformed or
    /// missing required fields.
    pub fn from_toml(raw: &str) -> Result<Self, Error> {
        toml::from_str(raw)
            .map_err(|e| Error::validation(format!("regex rule TOML: {e}"), "nvisy-pattern"))
    }
}
