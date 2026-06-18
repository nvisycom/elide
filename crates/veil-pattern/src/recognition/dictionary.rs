//! [`Dictionary`]: literal-term detection rule.

use derive_builder::Builder;
use serde::Deserialize;
use veil_core::entity::LabelRef;
use veil_core::primitive::{Confidence, CountryCode, LanguageTag};
use veil_core::{Error, ErrorKind};

use super::context::Context;
use super::term::Term;

/// Confidence policy for a [`Dictionary`]'s matches.
///
/// Either every term gets the same score ([`Uniform`]), or scores
/// vary by CSV source column ([`PerColumn`]). The untagged serde
/// representation accepts a bare number for the uniform case and
/// an array for the per-column case:
///
/// ```toml
/// score = 0.9              # Uniform
/// score = [0.85, 0.30]     # PerColumn
/// ```
///
/// [`Uniform`]: Scoring::Uniform
/// [`PerColumn`]: Scoring::PerColumn
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Scoring {
    /// One confidence stamped on every match — the common case.
    Uniform(Confidence),
    /// Per-column confidence vector. Entry `i` is the score for
    /// terms loaded from CSV column `i`. A term from a column past
    /// the end of this vector causes a recognizer-build error, so
    /// callers must declare one score per source column.
    PerColumn(Vec<Confidence>),
}

impl Scoring {
    /// Return `Ok(())` when the policy can resolve a score for at
    /// least one input.
    ///
    /// [`PerColumn`] with an empty vector can never resolve and is
    /// rejected here; the recognizer surfaces the error at build
    /// time.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason when the policy is invalid.
    ///
    /// [`PerColumn`]: Self::PerColumn
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Uniform(_) => Ok(()),
            Self::PerColumn(scores) if scores.is_empty() => {
                Err("PerColumn scoring with no scores can never resolve")
            }
            Self::PerColumn(_) => Ok(()),
        }
    }

    /// Resolve a score for the given source `column`.
    ///
    /// [`Uniform`] ignores `column` and always returns its score;
    /// [`PerColumn`] returns the entry at `column`, or `None` when
    /// `column` is `None` or out of range. Callers decide the
    /// fall-back policy (per-term override, hard error, …).
    ///
    /// [`Uniform`]: Self::Uniform
    /// [`PerColumn`]: Self::PerColumn
    #[must_use]
    pub fn get(&self, column: Option<u16>) -> Option<Confidence> {
        match self {
            Self::Uniform(s) => Some(*s),
            Self::PerColumn(scores) => column.and_then(|c| scores.get(c as usize).copied()),
        }
    }
}

impl Default for Scoring {
    fn default() -> Self {
        Self::Uniform(Confidence::MAX)
    }
}

/// Literal-term detection rule.
///
/// Scans for a fixed list of literals using a shared Aho-Corasick
/// automaton. Unlike [`Regex`], a dictionary has no regex engine,
/// no validator, and a [`Scoring`] policy shared across its terms.
///
/// # Examples
///
/// ```
/// use veil_core::entity::builtins;
/// use veil_pattern::{Dictionary, Term};
///
/// let dictionary = Dictionary::builder()
///     .with_name("nationalities")
///     .with_label(builtins::NATIONALITY.to_ref())
///     .with_terms(vec![
///         Term::new("German"),
///         Term::new("French"),
///         Term::new("Italian"),
///     ])
///     .build()
///     .expect("nationalities dictionary builds");
/// ```
///
/// [`Regex`]: crate::Regex
#[derive(Debug, Clone, PartialEq, Builder, Deserialize)]
#[builder(
    name = "DictionaryBuilder",
    pattern = "owned",
    setter(into, strip_option, prefix = "with"),
    build_fn(error = "Error")
)]
pub struct Dictionary {
    /// Human-readable identifier surfaced in trail provenance
    /// (e.g. `"nationalities"`).
    pub name: String,
    /// Entity label every match emits.
    pub label: LabelRef,
    /// Literal terms to scan for. Compiled into the shared
    /// Aho-Corasick automaton at recognizer-build time.
    pub terms: Vec<Term>,
    /// Confidence policy resolved against each term at
    /// recognizer-build time. Defaults to [`Scoring::Uniform`]
    /// with [`Confidence::MAX`].
    #[builder(default)]
    #[serde(default, rename = "score")]
    pub scoring: Scoring,
    /// Context keywords that lift confidence when one of them
    /// appears near a match. Either a flat list applied
    /// regardless of language, or a per-language map.
    #[builder(default)]
    #[serde(default)]
    pub context: Context,
    /// BCP-47 language tags the dictionary applies to. Empty means
    /// "any language"; otherwise the recognizer skips the
    /// dictionary when the per-call language hint is not in the
    /// list.
    #[builder(default)]
    #[serde(default)]
    pub languages: Vec<LanguageTag>,
    /// ISO 3166-1 alpha-2 country codes the dictionary applies
    /// to. Empty means "any country" — the dictionary fires
    /// regardless of the per-call jurisdiction hint.
    #[builder(default)]
    #[serde(default)]
    pub countries: Vec<CountryCode>,
    /// Require word-boundary surroundings on every match.
    ///
    /// With the default of `true`, the term `"am"` matches the
    /// word `"am"` but not the `"am"` inside `"example"`. Word
    /// characters are Unicode alphanumerics and `_`. Set to
    /// `false` to allow substring matches (e.g. scanning for
    /// embedded credentials).
    #[builder(default = "true")]
    #[serde(default = "default_word_boundary")]
    pub word_boundary: bool,
}

fn default_word_boundary() -> bool {
    true
}

impl Dictionary {
    /// Start a chainable builder.
    ///
    /// Required fields: `name`, `label`, `terms`.
    #[must_use]
    pub fn builder() -> DictionaryBuilder {
        DictionaryBuilder::default()
    }

    /// Parse a self-contained dictionary from a TOML source.
    ///
    /// The TOML must include a `terms` field; for metadata-only
    /// TOML paired with a separate term source, use
    /// [`metadata_from_toml`] instead.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the TOML is malformed or
    /// missing required fields.
    ///
    /// [`metadata_from_toml`]: Self::metadata_from_toml
    pub fn from_toml(raw: &str) -> Result<Self, Error> {
        toml::from_str(raw)
            .map_err(|e| Error::new(ErrorKind::Validation, format!("dictionary TOML: {e}")))
    }

    /// Parse dictionary metadata from a sidecar TOML source.
    ///
    /// The returned [`DictionaryBuilder`] is seeded with every
    /// field except `terms`; callers chain [`with_terms`] (e.g.
    /// loaded from a paired CSV/TXT) before [`build`].
    ///
    /// # Errors
    ///
    /// Returns a validation error when the TOML is malformed or
    /// missing required metadata fields.
    ///
    /// [`with_terms`]: DictionaryBuilder::with_terms
    /// [`build`]: DictionaryBuilder::build
    pub fn metadata_from_toml(raw: &str) -> Result<DictionaryBuilder, Error> {
        let metadata: DictionaryMetadata = toml::from_str(raw).map_err(|e| {
            Error::new(
                ErrorKind::Validation,
                format!("dictionary metadata TOML: {e}"),
            )
        })?;
        let mut builder = Dictionary::builder()
            .with_name(metadata.name)
            .with_label(metadata.label);
        if let Some(scoring) = metadata.score {
            builder = builder.with_scoring(scoring);
        }
        if let Some(context) = metadata.context {
            builder = builder.with_context(context);
        }
        if let Some(wb) = metadata.word_boundary {
            builder = builder.with_word_boundary(wb);
        }
        if let Some(languages) = metadata.languages {
            builder = builder.with_languages(languages);
        }
        if let Some(countries) = metadata.countries {
            builder = builder.with_countries(countries);
        }
        Ok(builder)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DictionaryMetadata {
    name: String,
    label: LabelRef,
    #[serde(default)]
    score: Option<Scoring>,
    #[serde(default)]
    context: Option<Context>,
    #[serde(default)]
    word_boundary: Option<bool>,
    #[serde(default)]
    languages: Option<Vec<LanguageTag>>,
    #[serde(default)]
    countries: Option<Vec<CountryCode>>,
}
