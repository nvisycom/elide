//! [`PatternRecognizer`] and its builder.

use aho_corasick::{AhoCorasick, MatchKind};
use regex::RegexSet;
use veil_context::{BoostRule, ContextEnhanced, Enhancer, SubstringMatcher};
use veil_core::entity::{Entity, LabelCatalog, LabelRef};
use veil_core::modality::text::Text;
use veil_core::primitive::LanguageTag;
use veil_core::recognition::{Recognizer, RecognizerId, RecognizerInput, RecognizerOutput};
use veil_core::{Error, ErrorKind, Result};

use super::compiled::{CompiledDictionary, CompiledPattern, has_word_boundaries};
use super::dictionary::Dictionary;
use super::regex::Regex;
use crate::shipped;
use crate::validators::{ValidationContext, ValidatorRegistry};

/// Runtime text recognizer composed of a regex pool and an
/// Aho-Corasick automaton.
///
/// Every registered [`Regex`] variant goes into one
/// [`::regex::RegexSet`] for a single one-pass scan across every
/// regex; every [`Dictionary`] term goes into one
/// [`::aho_corasick::AhoCorasick`] automaton for a single one-pass
/// scan across every literal. Both passes share one walk over the
/// input and emit entities in modality-local byte coordinates.
///
/// Construct via [`PatternRecognizer::builder`]. [`build`]
/// returns the bare recognizer; [`build_context_enhanced`] wraps
/// it in a [`ContextEnhanced`] layer that lifts confidence on
/// matches whose neighbourhood contains a per-label context
/// keyword.
///
/// # Examples
///
/// ```
/// use veil_pattern::PatternRecognizer;
///
/// let recognizer = PatternRecognizer::builder()
///     .with_builtin_patterns()
///     .with_builtin_dictionaries()
///     .build()
///     .expect("built-in recognizer builds");
/// ```
///
/// [`Regex`]: super::Regex
/// [`Dictionary`]: super::Dictionary
/// [`build`]: PatternRecognizerBuilder::build
/// [`build_context_enhanced`]: PatternRecognizerBuilder::build_context_enhanced
pub struct PatternRecognizer {
    patterns: Vec<CompiledPattern>,
    regex_set: Option<RegexSet>,
    dictionaries: Vec<CompiledDictionary>,
    aho: Option<AhoCorasick>,
}

impl PatternRecognizer {
    /// Start a chainable builder.
    ///
    /// A recognizer built with no patterns and no dictionaries is
    /// valid — it emits zero entities on every call.
    #[must_use]
    pub fn builder() -> PatternRecognizerBuilder {
        PatternRecognizerBuilder::default()
    }

    fn dictionary_owning_term(&self, term_id: usize) -> Option<&CompiledDictionary> {
        self.dictionaries
            .iter()
            .find(|d| term_id >= d.term_start && term_id < d.term_end)
    }
}

/// Accumulator of rules + validator registry for
/// [`PatternRecognizer`].
///
/// Patterns and dictionaries are stored as authored — compilation
/// into the pooled scanners happens in [`build`].
///
/// [`build`]: Self::build
#[derive(Debug, Clone, Default)]
pub struct PatternRecognizerBuilder {
    patterns: Vec<Regex>,
    dictionaries: Vec<Dictionary>,
    validators: Option<ValidatorRegistry>,
}

impl PatternRecognizerBuilder {
    /// Construct an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed with the shipped built-in patterns and
    /// dictionaries.
    ///
    /// Shorthand for
    /// `Self::new().with_builtin_patterns().with_builtin_dictionaries()`.
    #[must_use]
    pub fn builtin() -> Self {
        Self::new()
            .with_builtin_patterns()
            .with_builtin_dictionaries()
    }

    /// Register one pattern; patterns accumulate in registration
    /// order.
    #[must_use]
    pub fn with_pattern(mut self, pattern: Regex) -> Self {
        self.patterns.push(pattern);
        self
    }

    /// Register one dictionary; dictionaries accumulate in
    /// registration order.
    #[must_use]
    pub fn with_dictionary(mut self, dictionary: Dictionary) -> Self {
        self.dictionaries.push(dictionary);
        self
    }

    /// Register every shipped built-in pattern.
    #[must_use]
    pub fn with_builtin_patterns(mut self) -> Self {
        self.patterns.extend(shipped::patterns::all());
        self
    }

    /// Register every shipped built-in dictionary.
    #[must_use]
    pub fn with_builtin_dictionaries(mut self) -> Self {
        self.dictionaries.extend(shipped::dictionaries::all());
        self
    }

    /// Override the validator registry used to resolve variant
    /// validator names.
    ///
    /// Defaults to [`ValidatorRegistry::builtin`] when unset.
    #[must_use]
    pub fn with_validators(mut self, registry: ValidatorRegistry) -> Self {
        self.validators = Some(registry);
        self
    }

    /// Drop every pattern and dictionary whose label is not
    /// declared in `catalog`.
    ///
    /// The engine uses this to build a per-request recognizer from
    /// a workspace-wide template — rules that would emit labels no
    /// policy declared never run.
    #[must_use]
    pub fn filter_by_catalog(mut self, catalog: &LabelCatalog) -> Self {
        self.patterns.retain(|p| catalog.contains(&p.label));
        self.dictionaries.retain(|d| catalog.contains(&d.label));
        self
    }

    /// Return `true` when no patterns and no dictionaries are
    /// registered.
    ///
    /// The engine uses this to skip the per-request recognizer
    /// entirely after a catalog filter dropped every rule.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty() && self.dictionaries.is_empty()
    }

    /// Borrow the accumulated patterns.
    #[must_use]
    pub fn patterns(&self) -> &[Regex] {
        &self.patterns
    }

    /// Borrow the accumulated dictionaries.
    #[must_use]
    pub fn dictionaries(&self) -> &[Dictionary] {
        &self.dictionaries
    }

    /// Compile every rule into the pooled scanners and return the
    /// bare recognizer.
    ///
    /// Per-rule `context` keywords are ignored on the emission
    /// path; the recognizer emits raw confidence as authored by
    /// each rule. Wrap the result with [`build_context_enhanced`]
    /// (or compose with [`ContextEnhanced`] manually) to lift
    /// confidence on matches near a declared keyword.
    ///
    /// # Errors
    ///
    /// Returns a validation error when a pattern variant's regex
    /// fails to compile, when a variant references an unknown
    /// validator name, when a dictionary's `scoring` is invalid
    /// or under-declared for some term's source column, or when
    /// the shared automata cannot be constructed.
    ///
    /// [`build_context_enhanced`]: Self::build_context_enhanced
    pub fn build(self) -> Result<PatternRecognizer> {
        let validators = self
            .validators
            .clone()
            .unwrap_or_else(ValidatorRegistry::builtin);
        let (compiled_patterns, regex_set) = self.compile_patterns(&validators)?;
        let (compiled_dicts, aho) = self.compile_dictionaries()?;

        Ok(PatternRecognizer {
            patterns: compiled_patterns,
            regex_set,
            dictionaries: compiled_dicts,
            aho,
        })
    }

    /// Compile every rule and wrap the recognizer in a
    /// [`ContextEnhanced`] layer.
    ///
    /// Context keywords from every pattern and dictionary are
    /// harvested into per-label [`BoostRule`]s that lift confidence
    /// on matches whose neighbourhood contains a declared keyword.
    ///
    /// # Errors
    ///
    /// See [`build`].
    ///
    /// [`build`]: Self::build
    pub fn build_context_enhanced(self) -> Result<ContextEnhanced<PatternRecognizer>> {
        let enhancer = self.build_enhancer();
        let recognizer = self.build()?;
        Ok(ContextEnhanced::new(recognizer, enhancer))
    }

    /// Compile every `(pattern, variant)` pair into a
    /// [`CompiledPattern`] keyed by its slot in the shared
    /// [`RegexSet`].
    fn compile_patterns(
        &self,
        validators: &ValidatorRegistry,
    ) -> Result<(Vec<CompiledPattern>, Option<RegexSet>)> {
        let variant_total: usize = self.patterns.iter().map(|p| p.variants.len()).sum();
        let mut compiled = Vec::with_capacity(variant_total);
        let mut regex_sources = Vec::with_capacity(variant_total);

        for pattern in &self.patterns {
            for variant in &pattern.variants {
                let regex = ::regex::Regex::new(&variant.regex).map_err(|e| {
                    Error::new(ErrorKind::Validation, 
                        format!("pattern `{}`: invalid regex: {e}", pattern.name),
                    )
                })?;
                let validator = match variant.validator.as_deref() {
                    None => None,
                    Some(name) => Some(validators.resolve(name).ok_or_else(|| {
                        Error::new(ErrorKind::Validation, 
                            format!("pattern `{}`: unknown validator `{}`", pattern.name, name),
                        )
                    })?),
                };
                regex_sources.push(variant.regex.clone());
                compiled.push(CompiledPattern {
                    pattern_name: pattern.name.clone(),
                    label: pattern.label.clone(),
                    regex,
                    score: variant.score,
                    validator,
                    languages: pattern.languages.clone(),
                    countries: pattern.countries.clone(),
                });
            }
        }

        let regex_set = if regex_sources.is_empty() {
            None
        } else {
            Some(RegexSet::new(&regex_sources).map_err(|e| {
                Error::new(ErrorKind::Validation, format!("compiling regex set: {e}"))
            })?)
        };
        Ok((compiled, regex_set))
    }

    /// Compile every dictionary into a [`CompiledDictionary`]
    /// with its term-id range inside the shared Aho-Corasick
    /// automaton, plus per-term confidences resolved from the
    /// dictionary's `scoring` policy (with per-term overrides
    /// taking precedence).
    fn compile_dictionaries(&self) -> Result<(Vec<CompiledDictionary>, Option<AhoCorasick>)> {
        let mut compiled = Vec::with_capacity(self.dictionaries.len());
        let mut all_terms: Vec<String> = Vec::new();

        for dict in &self.dictionaries {
            if let Err(reason) = dict.scoring.validate() {
                return Err(Error::new(ErrorKind::Validation, 
                    format!("dictionary `{}`: {reason}", dict.name),
                ));
            }
            let term_start = all_terms.len();
            let mut term_scores = Vec::with_capacity(dict.terms.len());
            for entry in &dict.terms {
                all_terms.push(entry.term.clone());
                // Per-term `score` wins when set; otherwise ask
                // the dictionary's `Scoring` to resolve against
                // the term's source column. `None` means the
                // column didn't map to a declared score —
                // surfaced as a hard build error so silent
                // misconfiguration can't happen.
                let score = entry
                    .score
                    .or_else(|| dict.scoring.get(entry.column))
                    .ok_or_else(|| {
                        let column_desc = entry
                            .column
                            .map_or_else(|| "no column".to_owned(), |c| format!("column {c}"));
                        Error::new(ErrorKind::Validation, 
                            format!(
                                "dictionary `{}`: term `{}` ({column_desc}) has no score in \
                                 dictionary scoring",
                                dict.name, entry.term,
                            ),
                        )
                    })?;
                term_scores.push(score);
            }
            let term_end = all_terms.len();
            compiled.push(CompiledDictionary {
                name: dict.name.clone(),
                label: dict.label.clone(),
                term_start,
                term_end,
                term_scores,
                languages: dict.languages.clone(),
                countries: dict.countries.clone(),
                word_boundary: dict.word_boundary,
            });
        }

        let aho = if all_terms.is_empty() {
            None
        } else {
            Some(
                AhoCorasick::builder()
                    .ascii_case_insensitive(false)
                    // Longest-match-at-position: when both `en`
                    // and `English` start at the same offset,
                    // return `English`. Without this, the short
                    // ISO code would win and word-boundary
                    // post-filtering would then reject it,
                    // dropping the legitimate long-form match.
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(&all_terms)
                    .map_err(|e| {
                        Error::new(ErrorKind::Validation, 
                            format!("compiling dictionary automaton: {e}"),
                        )
                    })?,
            )
        };
        Ok((compiled, aho))
    }

    /// Build the wrapping [`Enhancer`] from per-pattern and
    /// per-dictionary context keywords.
    ///
    /// Per-rule [`Context`] produces one [`BoostRule`] per
    /// language scope (global rules carry
    /// `language = None`; per-language rules carry the language
    /// tag). The enhancer keys these by label and filters them
    /// against the per-call language hint at apply time.
    ///
    /// [`Context`]: super::Context
    fn build_enhancer(&self) -> Enhancer {
        let boost_rules: Vec<BoostRule> = self
            .context_keywords()
            .map(|(label, language, keywords)| {
                let rule = BoostRule::for_label(label.clone(), keywords.iter().cloned());
                match language {
                    Some(lang) => rule.with_language(lang.clone()),
                    None => rule,
                }
            })
            .collect();
        Enhancer::new(boost_rules, Box::new(SubstringMatcher))
    }

    /// Yield `(label, language, keywords)` for every pattern and
    /// dictionary that declares a non-empty context. Global
    /// keywords carry `language = None`; per-language keywords
    /// carry `Some(tag)`.
    fn context_keywords(
        &self,
    ) -> impl Iterator<Item = (&LabelRef, Option<&LanguageTag>, &[String])> {
        let pattern_keywords = self
            .patterns
            .iter()
            .filter(|p| !p.context.is_empty())
            .flat_map(|p| {
                p.context
                    .iter()
                    .map(move |(lang, kws)| (&p.label, lang, kws))
            });
        let dict_keywords = self
            .dictionaries
            .iter()
            .filter(|d| !d.context.is_empty())
            .flat_map(|d| {
                d.context
                    .iter()
                    .map(move |(lang, kws)| (&d.label, lang, kws))
            });
        pattern_keywords.chain(dict_keywords)
    }
}

impl Recognizer<Text> for PatternRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new("veil-pattern", env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(&self, input: &RecognizerInput<Text>) -> Result<RecognizerOutput<Text>> {
        let text = input.content.text.as_str();
        let mut entities: Vec<Entity<Text>> = Vec::new();

        if let Some(set) = self.regex_set.as_ref() {
            for pattern_id in set.matches(text).into_iter() {
                let pat = &self.patterns[pattern_id];
                if !input.applies_to_language(&pat.languages) {
                    continue;
                }
                if !input.applies_to_country(&pat.countries) {
                    continue;
                }
                let ctx = ValidationContext {
                    country: input.country,
                    language: input.language.clone(),
                };
                for m in pat.regex.find_iter(text) {
                    if let Some(validator) = pat.validator.as_ref()
                        && !validator.validate(m.as_str(), &ctx)
                    {
                        continue;
                    }
                    entities.push(pat.build_entity(m.start(), m.end()));
                }
            }
        }

        if let Some(aho) = self.aho.as_ref() {
            for mat in aho.find_iter(text) {
                let term_id = mat.pattern().as_usize();
                let Some(dict) = self.dictionary_owning_term(term_id) else {
                    continue;
                };
                if !input.applies_to_language(&dict.languages) {
                    continue;
                }
                if !input.applies_to_country(&dict.countries) {
                    continue;
                }
                if dict.word_boundary && !has_word_boundaries(text, mat.start(), mat.end()) {
                    continue;
                }
                let score = dict.term_scores[term_id - dict.term_start];
                entities.push(dict.build_entity(score, mat.start(), mat.end()));
            }
        }

        Ok(RecognizerOutput::new(entities))
    }
}
