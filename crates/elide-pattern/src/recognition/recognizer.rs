//! [`PatternRecognizer`] and its builder.

use std::result::Result as StdResult;

use aho_corasick::{AhoCorasick, MatchKind};
use elide_context::matching::SubstringMatcher;
use elide_context::{BoostRule, Enhanced, Enhancer};
use elide_core::entity::audit::AuditEvent;
use elide_core::entity::{Entity, LabelCatalog, LabelRef};
use elide_core::modality::TextRecognizable;
use elide_core::primitive::{Confidence, LanguageTag};
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, RecognizerId};
use elide_core::{Error, ErrorKind, Result};
// The external `regex` crate is aliased throughout because `Regex` is already
// this crate's rule type (`super::regex::Regex`, imported below).
use regex::{
    Error as CompiledRegexError, Regex as CompiledRegex, RegexBuilder, RegexSet, RegexSetBuilder,
};

use super::compiled::{CompiledDictionary, CompiledPattern, RawMatch, has_word_boundaries};
use super::context::Matching;
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
/// it in a [`Enhanced`] layer that lifts confidence on
/// matches whose neighbourhood contains a per-label context
/// keyword.
///
/// # Examples
///
/// ```
/// use elide_pattern::PatternRecognizer;
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

/// The label a rule emits under `catalog`: the most-specific candidate the
/// catalog declares, else the most-specific candidate overall.
///
/// A rule always emits *something* so the reconcile layers see every match and
/// can use a strong-but-out-of-catalog detection (a checksum-validated IBAN) to
/// subsume a weak in-catalog one nested inside it (a loose `drivers_license`
/// prefix). Restricting the output to the catalog is a downstream filter run
/// *after* reconciliation, so suppression evidence is not discarded before it
/// can be used. An empty catalog narrows nothing, so the most-specific
/// candidate wins.
fn resolve_label<'a>(candidates: &'a [LabelRef], catalog: &LabelCatalog) -> Option<&'a LabelRef> {
    candidates
        .iter()
        .find(|l| catalog.contains(l))
        .or_else(|| candidates.first())
}

/// One `(label, language)` context slice harvested from a pattern or
/// dictionary, ready to become a [`BoostRule`].
struct ContextKeywords<'a> {
    /// The label the keywords boost.
    label: &'a LabelRef,
    /// Language scope; `None` is language-agnostic.
    language: Option<&'a LanguageTag>,
    /// The keyword literals to search for near a match.
    keywords: &'a [String],
    /// Whole-word vs substring matching for these keywords.
    matching: Matching,
    /// Additive boost override from the `[context]` table, or `None` to use
    /// the enhancer default.
    boost: Option<f32>,
}

/// Accumulator of rules + validator registry for [`PatternRecognizer`].
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
    /// Compiled-automaton byte budget applied to every variant regex and
    /// to the shared [`RegexSet`]. `None` leaves the `regex` crate default
    /// (~10 MB) in place.
    size_limit: Option<usize>,
    /// Lazy-DFA cache byte budget applied likewise. `None` leaves the
    /// `regex` crate default in place.
    dfa_size_limit: Option<usize>,
    /// Cap on the total number of dictionary terms across every registered
    /// dictionary (the Aho-Corasick automaton is shared). `None` is
    /// unbounded.
    term_count_limit: Option<usize>,
    /// Cap on the total bytes of all dictionary terms across every
    /// dictionary. `None` is unbounded.
    term_bytes_limit: Option<usize>,
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

    /// Cap the compiled-automaton size, in bytes, of every variant regex
    /// **and** of the shared [`RegexSet`] union.
    ///
    /// A caller can bound each regex *source* it supplies, but many
    /// individually-small sources still union into one large `RegexSet`
    /// whose compiled size can only be bounded here. [`build`] fails with a
    /// validation error when a regex or the union exceeds the limit.
    ///
    /// Unset by default — the `regex` crate's own default budget (~10 MB)
    /// applies, so behavior is unchanged unless a limit is set.
    ///
    /// [`build`]: Self::build
    #[must_use]
    pub fn with_size_limit(mut self, bytes: usize) -> Self {
        self.size_limit = Some(bytes);
        self
    }

    /// Cap the lazy-DFA cache size, in bytes, of every variant regex and the
    /// shared [`RegexSet`]. This bounds match-time memory (the DFA is built
    /// on demand while scanning) rather than compile-time automaton size.
    ///
    /// Unset by default — the `regex` crate's own default applies.
    #[must_use]
    pub fn with_dfa_size_limit(mut self, bytes: usize) -> Self {
        self.dfa_size_limit = Some(bytes);
        self
    }

    /// Cap the total number of dictionary terms across **every** registered
    /// dictionary — they compile into one shared Aho-Corasick automaton, so
    /// the limit is a recognizer-wide aggregate, not per-dictionary.
    ///
    /// Dictionaries are literal-match (no regex backtracking surface); this
    /// bounds compile cost and automaton memory, not a match-time hazard.
    /// [`build`] fails with a validation error when the total is exceeded.
    ///
    /// Unbounded by default.
    ///
    /// [`build`]: Self::build
    #[must_use]
    pub fn with_term_count_limit(mut self, max: usize) -> Self {
        self.term_count_limit = Some(max);
        self
    }

    /// Cap the total bytes of all dictionary terms across every dictionary —
    /// a finer proxy for automaton size than raw term count. [`build`] fails
    /// with a validation error when the sum of term lengths exceeds `max`.
    ///
    /// Unbounded by default.
    ///
    /// [`build`]: Self::build
    #[must_use]
    pub fn with_term_bytes_limit(mut self, max: usize) -> Self {
        self.term_bytes_limit = Some(max);
        self
    }

    /// Drop every pattern and dictionary none of whose candidate labels are
    /// declared in `catalog`.
    ///
    /// The engine uses this to build a per-request recognizer from a
    /// workspace-wide template — rules that could emit no label any policy
    /// declared never run. A rule is kept when *any* of its candidates is in
    /// the catalog; at recognize time it emits the first one that is.
    #[must_use]
    pub fn filter_by_catalog(mut self, catalog: &LabelCatalog) -> Self {
        self.patterns
            .retain(|p| p.labels.iter().any(|l| catalog.contains(l)));
        self.dictionaries
            .retain(|d| d.labels.iter().any(|l| catalog.contains(l)));
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
    /// (or compose with [`Enhanced`] manually) to lift
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

    /// Compile every rule and wrap the recognizer in an [`Enhanced`] layer
    /// that runs keyword-boost context enhancement.
    ///
    /// Context keywords from every pattern and dictionary are harvested into
    /// per-label [`BoostRule`]s that lift confidence on matches whose
    /// neighbourhood contains a declared keyword. Because enhancement runs on
    /// the stream-positioned drafts before they are lifted, this works for
    /// every modality (text, tabular, image, audio).
    ///
    /// # Errors
    ///
    /// See [`build`].
    ///
    /// [`build`]: Self::build
    pub fn build_context_enhanced(self) -> Result<Enhanced<PatternRecognizer>> {
        let enhancer = self.build_enhancer();
        let recognizer = self.build()?;
        Ok(Enhanced::new(recognizer, enhancer))
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
                let regex = self.compile_regex(&variant.regex).map_err(|e| {
                    Error::new(
                        ErrorKind::Configuration,
                        format!("pattern `{}`: invalid regex: {e}", pattern.name),
                    )
                })?;
                let validator = match variant.validator.as_deref() {
                    None => None,
                    Some(name) => Some(validators.resolve(name).ok_or_else(|| {
                        Error::new(
                            ErrorKind::Configuration,
                            format!("pattern `{}`: unknown validator `{}`", pattern.name, name),
                        )
                    })?),
                };
                regex_sources.push(variant.regex.clone());
                compiled.push(CompiledPattern {
                    pattern_name: pattern.name.clone(),
                    labels: pattern.labels.clone(),
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
            let mut builder = RegexSetBuilder::new(&regex_sources);
            if let Some(bytes) = self.size_limit {
                builder.size_limit(bytes);
            }
            if let Some(bytes) = self.dfa_size_limit {
                builder.dfa_size_limit(bytes);
            }
            Some(builder.build().map_err(|e| {
                Error::new(
                    ErrorKind::Configuration,
                    format!("compiling regex set: {e}"),
                )
            })?)
        };
        Ok((compiled, regex_set))
    }

    /// Compile one regex source, honoring the builder's optional
    /// [`size_limit`]/[`dfa_size_limit`]. Unset limits leave the `regex`
    /// crate defaults untouched, so an unconfigured builder compiles exactly
    /// as before.
    ///
    /// [`size_limit`]: Self::with_size_limit
    /// [`dfa_size_limit`]: Self::with_dfa_size_limit
    fn compile_regex(&self, source: &str) -> StdResult<CompiledRegex, CompiledRegexError> {
        let mut builder = RegexBuilder::new(source);
        if let Some(bytes) = self.size_limit {
            builder.size_limit(bytes);
        }
        if let Some(bytes) = self.dfa_size_limit {
            builder.dfa_size_limit(bytes);
        }
        builder.build()
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
                return Err(Error::new(
                    ErrorKind::Configuration,
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
                        Error::new(
                            ErrorKind::Configuration,
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
                labels: dict.labels.clone(),
                term_start,
                term_end,
                term_scores,
                languages: dict.languages.clone(),
                countries: dict.countries.clone(),
                word_boundary: dict.word_boundary,
            });
        }

        // Bound the shared automaton before building it: the count and the
        // total term bytes are recognizer-wide aggregates across every
        // dictionary. Unset limits skip the check.
        if let Some(max) = self.term_count_limit
            && all_terms.len() > max
        {
            return Err(Error::new(
                ErrorKind::Configuration,
                format!(
                    "dictionary term count {} exceeds limit {max}",
                    all_terms.len()
                ),
            ));
        }
        if let Some(max) = self.term_bytes_limit {
            let total: usize = all_terms.iter().map(String::len).sum();
            if total > max {
                return Err(Error::new(
                    ErrorKind::Configuration,
                    format!("dictionary term bytes {total} exceeds limit {max}"),
                ));
            }
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
                        Error::new(
                            ErrorKind::Configuration,
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
        // Inline keyword context (Global / PerLanguage lists, and any inline
        // keywords a `Sourced` context carries).
        let mut boost_rules: Vec<BoostRule> = self
            .context_keywords()
            .map(|ck| {
                let mut rule = BoostRule::new(ck.label.clone(), ck.keywords.iter().cloned())
                    .with_word_boundary(ck.matching == Matching::Word);
                if let Some(boost) = ck.boost {
                    rule = rule.with_boost(Confidence::clamped(boost));
                }
                match ck.language {
                    Some(lang) => rule.with_language(lang.clone()),
                    None => rule,
                }
            })
            .collect();

        // Dictionary-sourced context: a pattern whose `[context]` names
        // dictionaries borrows their terms as boost keywords for its own
        // labels — e.g. a `monetary_amount` pattern lifts a number beside any
        // currency name from the `currencies` dictionary.
        for pattern in self.patterns.iter() {
            let names = pattern.context.dictionaries();
            if names.is_empty() {
                continue;
            }
            let terms: Vec<String> = self
                .dictionaries
                .iter()
                .filter(|d| names.contains(&d.name))
                .flat_map(|d| d.terms.iter().map(|t| t.term.clone()))
                .collect();
            if terms.is_empty() {
                continue;
            }
            let word_boundary = pattern.context.matching() == Matching::Word;
            let boost = pattern.context.boost();
            for label in &pattern.labels {
                let mut rule = BoostRule::new(label.clone(), terms.iter().cloned())
                    .with_word_boundary(word_boundary);
                if let Some(boost) = boost {
                    rule = rule.with_boost(Confidence::clamped(boost));
                }
                boost_rules.push(rule);
            }
        }

        Enhancer::new(boost_rules, SubstringMatcher)
    }

    /// Yield one [`ContextKeywords`] for every `(label, language)` context a
    /// pattern or dictionary declares.
    ///
    /// A rule with several candidate labels contributes its keywords under
    /// *each* candidate, so whichever label the request catalog resolves the
    /// match to still gets its neighbourhood boost.
    fn context_keywords(&self) -> impl Iterator<Item = ContextKeywords<'_>> {
        let pattern_keywords = self
            .patterns
            .iter()
            .filter(|p| !p.context.is_empty())
            .flat_map(|p| {
                let matching = p.context.matching();
                let boost = p.context.boost();
                p.context.iter().flat_map(move |(language, keywords)| {
                    p.labels.iter().map(move |label| ContextKeywords {
                        label,
                        language,
                        keywords,
                        matching,
                        boost,
                    })
                })
            });
        let dict_keywords = self
            .dictionaries
            .iter()
            .filter(|d| !d.context.is_empty())
            .flat_map(|d| {
                let matching = d.context.matching();
                let boost = d.context.boost();
                d.context.iter().flat_map(move |(language, keywords)| {
                    d.labels.iter().map(move |label| ContextKeywords {
                        label,
                        language,
                        keywords,
                        matching,
                        boost,
                    })
                })
            });
        pattern_keywords.chain(dict_keywords)
    }
}

impl PatternRecognizer {
    /// Place a match at `range` of the recognized text into a located
    /// [`Entity`], keeping the range as the entity's `recognized_range`.
    /// Drops the match (`None`) when its range can't be placed in the medium
    /// (an OCR/transcript range no enrichment covers).
    fn build_entity<M: TextRecognizable>(
        &self,
        raw: RawMatch,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Option<Entity<M>> {
        let location = M::locate(raw.range.clone(), data, ctx.artifact())?;
        let event = AuditEvent::pattern("pattern", raw.confidence, location.clone(), raw.pattern);
        Some(
            Entity::builder()
                .with_label(raw.label)
                .with_location(location)
                .with_confidence(raw.confidence)
                .with_recognized_range(raw.range)
                .with_event(event)
                .build()
                .expect("required fields provided"),
        )
    }
}

#[async_trait::async_trait]
impl<M: TextRecognizable> Recognizer<M> for PatternRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new("elide-pattern", env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Recognition<M>> {
        let text = M::as_text(data, ctx.artifact());
        let mut entities: Vec<Entity<M>> = Vec::new();

        if let Some(set) = self.regex_set.as_ref() {
            for pattern_id in set.matches(text).into_iter() {
                let pat = &self.patterns[pattern_id];
                // Locale filtering keys on *asserted* languages and countries
                // only. A detected language never filters: detection is
                // unreliable on short, word-poor input (a single data cell like
                // `12345678Z` has no language, yet resolves to an arbitrary one)
                // and would wrongly suppress a valid locale-scoped match. When
                // the caller asserts a language or country, that assertion is
                // authoritative and gates the locale patterns.
                if !ctx.applies_to_asserted_language(&pat.languages) {
                    continue;
                }
                if !ctx.applies_to_country(&pat.countries) {
                    continue;
                }
                // The label this rule emits under the request catalog. `None`
                // when the caller enabled none of its candidates, so the rule
                // contributes nothing this call.
                let Some(label) = resolve_label(&pat.labels, ctx.catalog()) else {
                    continue;
                };
                let validation_ctx = ValidationContext {
                    countries: ctx.scope().countries.clone(),
                    language: ctx.primary_language().cloned(),
                };
                for m in pat.regex.find_iter(text) {
                    if let Some(validator) = pat.validator.as_ref()
                        && !validator.validate(m.as_str(), &validation_ctx)
                    {
                        continue;
                    }
                    if let Some(entity) =
                        self.build_entity::<M>(pat.raw_match(label.clone(), m.range()), data, ctx)
                    {
                        entities.push(entity);
                    }
                }
            }
        }

        if let Some(aho) = self.aho.as_ref() {
            for mat in aho.find_iter(text) {
                let term_id = mat.pattern().as_usize();
                let Some(dict) = self.dictionary_owning_term(term_id) else {
                    continue;
                };
                // Asserted-language + country locale filter (see the pattern
                // loop above); a detected language never suppresses a match.
                if !ctx.applies_to_asserted_language(&dict.languages) {
                    continue;
                }
                if !ctx.applies_to_country(&dict.countries) {
                    continue;
                }
                let Some(label) = resolve_label(&dict.labels, ctx.catalog()) else {
                    continue;
                };
                let range = mat.range();
                if dict.word_boundary && !has_word_boundaries(text, range.clone()) {
                    continue;
                }
                let score = dict.term_scores[term_id - dict.term_start];
                if let Some(entity) =
                    self.build_entity::<M>(dict.raw_match(label.clone(), score, range), data, ctx)
                {
                    entities.push(entity);
                }
            }
        }

        // Pattern matching is pure-CPU: no model usage to report.
        Ok(entities.into())
    }
}
