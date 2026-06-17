//! [`PatternRecognizer`] and its builder.

use aho_corasick::{AhoCorasick, MatchKind};
use nvisy_context::{BoostRule, ContextEnhanced, Enhancer, SubstringMatcher};
use nvisy_core::entity::{Entity, EntityLabelCatalog, EntityLabelRef};
use nvisy_core::modality::Text;
use nvisy_core::primitive::LanguageTag;
use nvisy_core::recognition::{EntityRecognizer, RecognizerInput, RecognizerOutput};
use nvisy_core::{Error, Result};
use regex::RegexSet;

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
/// use nvisy_pattern::PatternRecognizer;
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
    pub fn filter_by_catalog(mut self, catalog: &EntityLabelCatalog) -> Self {
        self.patterns
            .retain(|p| catalog.lookup(p.label.as_str()).is_some());
        self.dictionaries
            .retain(|d| catalog.lookup(d.label.as_str()).is_some());
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
                    Error::validation(
                        format!("pattern `{}`: invalid regex: {e}", pattern.name),
                        "nvisy-pattern",
                    )
                })?;
                let validator = match variant.validator.as_deref() {
                    None => None,
                    Some(name) => Some(validators.resolve(name).ok_or_else(|| {
                        Error::validation(
                            format!("pattern `{}`: unknown validator `{}`", pattern.name, name),
                            "nvisy-pattern",
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
                Error::validation(format!("compiling regex set: {e}"), "nvisy-pattern")
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
                return Err(Error::validation(
                    format!("dictionary `{}`: {reason}", dict.name),
                    "nvisy-pattern",
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
                        Error::validation(
                            format!(
                                "dictionary `{}`: term `{}` ({column_desc}) has no score in \
                                 dictionary scoring",
                                dict.name, entry.term,
                            ),
                            "nvisy-pattern",
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
                        Error::validation(
                            format!("compiling dictionary automaton: {e}"),
                            "nvisy-pattern",
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
    ) -> impl Iterator<Item = (&EntityLabelRef, Option<&LanguageTag>, &[String])> {
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

#[async_trait::async_trait]
impl EntityRecognizer<Text> for PatternRecognizer {
    async fn recognize(&self, input: &RecognizerInput<Text>) -> Result<RecognizerOutput<Text>> {
        let text = input.data.text.as_str();
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nvisy_core::entity::{Entity, EntityLabelRef, builtins};
    use nvisy_core::modality::{Text, TextData};
    use nvisy_core::primitive::{Confidence, CountryCode};
    use nvisy_core::recognition::RecognizerInput;

    use super::*;
    use crate::Dictionary;
    use crate::recognition::term::Term;

    fn dict(name: &str, terms: &[&str], word_boundary: bool) -> Dictionary {
        Dictionary::builder()
            .with_name(name.to_owned())
            .with_label(EntityLabelRef::from(builtins::LANGUAGE.name.clone()))
            .with_terms(terms.iter().copied().map(Term::new).collect::<Vec<_>>())
            .with_word_boundary(word_boundary)
            .build()
            .expect("dictionary builds")
    }

    async fn run(recognizer: &impl EntityRecognizer<Text>, text: &str) -> Vec<Entity<Text>> {
        let input = RecognizerInput::new(TextData::new(text.to_owned()));
        recognizer
            .recognize(&input)
            .await
            .expect("recognize succeeds")
            .entities
    }

    #[tokio::test]
    async fn word_boundary_rejects_substring_matches() {
        let recognizer = PatternRecognizer::builder()
            .with_dictionary(dict("langs", &["am", "or"], true))
            .build()
            .expect("recognizer builds");

        let entities = run(&recognizer, "the example or a candidate").await;
        let matched: Vec<&str> = entities
            .iter()
            .map(|e| &"the example or a candidate"[e.location.start..e.location.end])
            .collect();

        // "am" inside "example" and "or" inside "candidate" are
        // substring matches and must be rejected. The standalone
        // "or" between two spaces must be kept.
        assert_eq!(matched, vec!["or"]);
    }

    #[tokio::test]
    async fn word_boundary_disabled_keeps_substring_matches() {
        let recognizer = PatternRecognizer::builder()
            .with_dictionary(dict("langs", &["am"], false))
            .build()
            .expect("recognizer builds");

        let entities = run(&recognizer, "example").await;
        assert_eq!(entities.len(), 1, "substring match must be kept");
    }

    #[test]
    fn regex_parses_flat_context_as_global() {
        let toml = r#"
            name = "x"
            label = "government_id"
            context = ["ssn", "social security"]
            [[variants]]
            regex = "\\d+"
        "#;
        let regex = crate::Regex::from_toml(toml).expect("flat-context TOML parses");
        assert!(matches!(regex.context, crate::Context::Global(_)));
    }

    #[test]
    fn regex_parses_table_context_as_per_language() {
        let toml = r#"
            name = "x"
            label = "payment_card"
            [context]
            en = ["card", "credit"]
            es = ["tarjeta", "crédito"]
            [[variants]]
            regex = "\\d+"
        "#;
        let regex = crate::Regex::from_toml(toml).expect("table-context TOML parses");
        let map = match regex.context {
            crate::Context::PerLanguage(m) => m,
            _ => panic!("expected PerLanguage"),
        };
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn regex_omits_countries_by_default() {
        let toml = r#"
            name = "x"
            label = "government_id"
            [[variants]]
            regex = "\\d+"
        "#;
        let regex = crate::Regex::from_toml(toml).expect("TOML parses");
        assert!(
            regex.countries.is_empty(),
            "default countries must be empty"
        );
    }

    #[test]
    fn regex_parses_countries_field() {
        let toml = r#"
            name = "ssn"
            label = "government_id"
            countries = ["US"]
            [[variants]]
            regex = "\\d+"
        "#;
        let regex = crate::Regex::from_toml(toml).expect("TOML parses");
        assert_eq!(regex.countries.len(), 1);
        assert_eq!(regex.countries[0].as_str(), "US");
    }

    #[test]
    fn regex_parses_multiple_countries() {
        let toml = r#"
            name = "eu-vat"
            label = "tax_id"
            countries = ["de", "FR", "iT"]
            [[variants]]
            regex = "\\d+"
        "#;
        let regex = crate::Regex::from_toml(toml).expect("TOML parses");
        assert_eq!(regex.countries.len(), 3);
        // Construction normalises to uppercase.
        let codes: Vec<&str> = regex.countries.iter().map(CountryCode::as_str).collect();
        assert_eq!(codes, vec!["DE", "FR", "IT"]);
    }

    #[test]
    fn regex_rejects_invalid_country() {
        let toml = r#"
            name = "x"
            label = "government_id"
            countries = ["XZ"]
            [[variants]]
            regex = "\\d+"
        "#;
        assert!(
            crate::Regex::from_toml(toml).is_err(),
            "unassigned country code must error",
        );
    }

    #[test]
    fn regex_builder_accepts_countries() {
        let variant = crate::Variant::new(r"\d{3}-\d{2}-\d{4}").unwrap();
        let regex = crate::Regex::builder()
            .with_name("ssn")
            .with_label(builtins::GOVERNMENT_ID.label_ref())
            .with_variants(vec![variant])
            .with_countries(vec![CountryCode::new("US").unwrap()])
            .build()
            .expect("regex builds");
        assert_eq!(regex.countries.len(), 1);
        assert_eq!(regex.countries[0].as_str(), "US");
    }

    async fn run_with_language(
        recognizer: &impl EntityRecognizer<Text>,
        text: &str,
        language: Option<&str>,
    ) -> Vec<Entity<Text>> {
        let mut input = RecognizerInput::new(TextData::new(text.to_owned()));
        if let Some(lang) = language {
            input = input.with_language(LanguageTag::new(lang).expect("language tag parses"));
        }
        recognizer
            .recognize(&input)
            .await
            .expect("recognize succeeds")
            .entities
    }

    async fn run_with_country(
        recognizer: &impl EntityRecognizer<Text>,
        text: &str,
        country: Option<&str>,
    ) -> Vec<Entity<Text>> {
        let mut input = RecognizerInput::new(TextData::new(text.to_owned()));
        if let Some(c) = country {
            input = input.with_country(CountryCode::new(c).expect("country code parses"));
        }
        recognizer
            .recognize(&input)
            .await
            .expect("recognize succeeds")
            .entities
    }

    fn us_ssn_regex() -> crate::Regex {
        let variant = crate::Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
            .expect("variant builds")
            .with_score(Confidence::clamped(0.5));
        crate::Regex::builder()
            .with_name("ssn")
            .with_label(builtins::GOVERNMENT_ID.label_ref())
            .with_variants(vec![variant])
            .with_countries(vec![CountryCode::new("US").unwrap()])
            .build()
            .expect("regex builds")
    }

    #[tokio::test]
    async fn country_scoped_rule_fires_under_matching_hint() {
        let recognizer = PatternRecognizer::builder()
            .with_pattern(us_ssn_regex())
            .build()
            .expect("recognizer builds");
        let entities = run_with_country(&recognizer, "SSN: 123-45-6789", Some("US")).await;
        assert_eq!(entities.len(), 1, "US-scoped rule must fire under US hint");
    }

    #[tokio::test]
    async fn country_scoped_rule_skipped_under_non_matching_hint() {
        let recognizer = PatternRecognizer::builder()
            .with_pattern(us_ssn_regex())
            .build()
            .expect("recognizer builds");
        let entities = run_with_country(&recognizer, "SSN: 123-45-6789", Some("GB")).await;
        assert!(
            entities.is_empty(),
            "US-scoped rule must not fire under GB hint",
        );
    }

    #[tokio::test]
    async fn country_scoped_rule_fires_without_hint() {
        // Permissive fallback: missing hint shouldn't drop the
        // detection. Matches the existing `applies_to_language`
        // semantic.
        let recognizer = PatternRecognizer::builder()
            .with_pattern(us_ssn_regex())
            .build()
            .expect("recognizer builds");
        let entities = run_with_country(&recognizer, "SSN: 123-45-6789", None).await;
        assert_eq!(
            entities.len(),
            1,
            "missing country hint must permit US-scoped rule to run",
        );
    }

    fn per_language_credit_card_regex() -> crate::Regex {
        let variant = crate::Variant::new(r"\b\d{16}\b")
            .expect("variant builds")
            .with_score(Confidence::clamped(0.5));
        let mut context = HashMap::new();
        context.insert(
            LanguageTag::new("en").unwrap(),
            vec!["credit".to_owned(), "card".to_owned()],
        );
        context.insert(
            LanguageTag::new("es").unwrap(),
            vec!["tarjeta".to_owned(), "crédito".to_owned()],
        );
        crate::Regex::builder()
            .with_name("credit_card")
            .with_label(builtins::PAYMENT_CARD.label_ref())
            .with_context(crate::Context::PerLanguage(context))
            .with_variants(vec![variant])
            .build()
            .expect("regex builds")
    }

    #[tokio::test]
    async fn per_language_boost_fires_for_matching_language() {
        let recognizer = PatternRecognizer::builder()
            .with_pattern(per_language_credit_card_regex())
            .build_context_enhanced()
            .expect("recognizer builds");

        let text = "Pay with your credit card 4111111111111111 today";
        let entities = run_with_language(&recognizer, text, Some("en")).await;
        let card = entities
            .iter()
            .find(|e| &text[e.location.start..e.location.end] == "4111111111111111")
            .expect("card match present");
        assert!(
            card.confidence.get() > 0.5,
            "English keyword `credit` should boost under en hint",
        );
    }

    #[tokio::test]
    async fn per_language_boost_fires_for_regional_variant() {
        // Pattern is scoped `en`; hint is `en-US`. Primary subtag
        // matches, so the boost must fire.
        let recognizer = PatternRecognizer::builder()
            .with_pattern(per_language_credit_card_regex())
            .build_context_enhanced()
            .expect("recognizer builds");

        let text = "Pay with your credit card 4111111111111111 today";
        let entities = run_with_language(&recognizer, text, Some("en-US")).await;
        let card = entities
            .iter()
            .find(|e| &text[e.location.start..e.location.end] == "4111111111111111")
            .expect("card match present");
        assert!(
            card.confidence.get() > 0.5,
            "`en-US` hint should fire the `en`-scoped boost",
        );
    }

    #[tokio::test]
    async fn rule_language_filter_accepts_regional_variant() {
        // Pattern is scoped `languages = ["en"]`; the per-call
        // hint is `en-US`. The rule must still run.
        let variant = crate::Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
            .expect("variant builds")
            .with_score(Confidence::clamped(0.5));
        let regex = crate::Regex::builder()
            .with_name("ssn")
            .with_label(builtins::GOVERNMENT_ID.label_ref())
            .with_variants(vec![variant])
            .with_languages(vec![LanguageTag::new("en").unwrap()])
            .build()
            .expect("regex builds");

        let recognizer = PatternRecognizer::builder()
            .with_pattern(regex)
            .build()
            .expect("recognizer builds");

        let entities = run_with_language(&recognizer, "SSN: 123-45-6789", Some("en-US")).await;
        assert_eq!(
            entities.len(),
            1,
            "`en`-scoped rule must run for `en-US` input",
        );
    }

    #[tokio::test]
    async fn per_language_boost_skipped_for_non_matching_language() {
        let recognizer = PatternRecognizer::builder()
            .with_pattern(per_language_credit_card_regex())
            .build_context_enhanced()
            .expect("recognizer builds");

        // English keywords near the match, but caller asserted Spanish.
        let text = "Pay with your credit card 4111111111111111 today";
        let entities = run_with_language(&recognizer, text, Some("es")).await;
        let card = entities
            .iter()
            .find(|e| &text[e.location.start..e.location.end] == "4111111111111111")
            .expect("card match present");
        assert!(
            (card.confidence.get() - 0.5).abs() < f64::EPSILON,
            "English keywords must not boost under es hint",
        );
    }

    #[tokio::test]
    async fn no_language_hint_unions_per_language_keywords() {
        let recognizer = PatternRecognizer::builder()
            .with_pattern(per_language_credit_card_regex())
            .build_context_enhanced()
            .expect("recognizer builds");

        // English keyword near the match, no language hint set.
        let text = "Pay with your credit card 4111111111111111 today";
        let entities = run_with_language(&recognizer, text, None).await;
        let card = entities
            .iter()
            .find(|e| &text[e.location.start..e.location.end] == "4111111111111111")
            .expect("card match present");
        assert!(
            card.confidence.get() > 0.5,
            "missing language hint should permit any per-language keyword to boost",
        );
    }
}
