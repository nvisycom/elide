//! [`PatternRecognizerBuilder`]: collects rules and compiles them into a
//! [`PatternRecognizer`].

use std::collections::HashSet;
use std::mem;
use std::result::Result as StdResult;
use std::sync::Arc;

use aho_corasick::{AhoCorasick, MatchKind};
use elide_core::entity::LabelCatalog;
use elide_core::{Error, ErrorKind, Result};
use regex::{Error as RegexError, Regex, RegexBuilder, RegexSet, RegexSetBuilder};

use super::PatternRecognizer;
use crate::context::matching::{KeywordMatcher, SubstringMatcher};
use crate::context::{CompiledContext, RuleContext};
use crate::rule::{
    CompiledDictionary, CompiledPattern, CompiledWalk, Dictionary, Pattern, Variant, Walk,
};
use crate::shipped;
use crate::validators::ValidatorRegistry;

/// Byte budgets for compiling a regex, mirroring the `regex` crate's two knobs.
///
/// Applied in two places, independently: once per *individual* variant
/// ([`with_variant_budget`]) and once for the shared `RegexSet` *union*
/// ([`with_union_budget`]). A field left `None` leaves the crate default in
/// place.
///
/// [`with_variant_budget`]: PatternRecognizerBuilder::with_variant_budget
/// [`with_union_budget`]: PatternRecognizerBuilder::with_union_budget
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegexBudget {
    /// Compiled-automaton (NFA) byte budget — bounds *compile-time* size.
    pub nfa_size: Option<usize>,
    /// Lazy-DFA cache byte budget — bounds *match-time* memory (the DFA is
    /// built on demand while scanning).
    pub dfa_size: Option<usize>,
}

impl RegexBudget {
    /// A budget capping the compiled-automaton (NFA) size to `bytes`, leaving
    /// the DFA cache at the crate default. The common case.
    #[must_use]
    pub fn nfa(bytes: usize) -> Self {
        Self {
            nfa_size: Some(bytes),
            dfa_size: None,
        }
    }
}

/// Accumulator of rules + validator registry for [`PatternRecognizer`].
///
/// Patterns, dictionaries, and walks are stored as authored, compilation
/// into the pooled scanners happens in [`build`].
///
/// [`build`]: Self::build
#[derive(Debug, Clone, Default)]
pub struct PatternRecognizerBuilder {
    patterns: Vec<Pattern>,
    dictionaries: Vec<Dictionary>,
    /// Dictionaries [`filter_by_catalog`] dropped that a kept rule's context
    /// still draws keywords from. They feed context only, never recognition.
    ///
    /// [`filter_by_catalog`]: Self::filter_by_catalog
    context_sources: Vec<Dictionary>,
    walks: Vec<Walk>,
    validators: Option<ValidatorRegistry>,
    /// Byte budget for each *individual* variant regex. This is the knob to
    /// bound a caller-supplied rule: a legitimate single regex compiles well
    /// under 1 MiB, so a tight cap here rejects a pathological source at build
    /// without touching the (necessarily larger) union below.
    variant_budget: RegexBudget,
    /// Byte budget for the shared [`RegexSet`] union (every registered variant
    /// compiled together). The shipped builtins union past several MB, so keep
    /// this generous — a single budget cannot bound both a tight per-variant cap
    /// and the large union.
    union_budget: RegexBudget,
    /// Cap on the total number of dictionary terms across every registered
    /// dictionary (the Aho-Corasick automaton is shared). `None` is
    /// unbounded.
    term_count_limit: Option<usize>,
    /// Cap on the total bytes of all dictionary terms across every
    /// dictionary. `None` is unbounded.
    term_bytes_limit: Option<usize>,
    /// How context keywords are matched; `None` is [`SubstringMatcher`].
    keyword_matcher: Option<Arc<dyn KeywordMatcher>>,
}

impl PatternRecognizerBuilder {
    /// Construct an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed with the shipped built-in patterns, dictionaries, and
    /// walks.
    ///
    /// Shorthand for `Self::new().with_builtin_patterns()
    /// .with_builtin_dictionaries().with_builtin_walks()`.
    #[must_use]
    pub fn builtin() -> Self {
        Self::new()
            .with_builtin_patterns()
            .with_builtin_dictionaries()
            .with_builtin_walks()
    }

    /// Register one pattern; patterns accumulate in registration
    /// order.
    #[must_use]
    pub fn with_pattern(mut self, pattern: Pattern) -> Self {
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

    /// Register one walk; walks accumulate in registration order.
    #[must_use]
    pub fn with_walk(mut self, walk: Walk) -> Self {
        self.walks.push(walk);
        self
    }

    /// Register every shipped built-in pattern.
    #[must_use]
    pub fn with_builtin_patterns(mut self) -> Self {
        self.patterns.extend(shipped::patterns::all());
        self
    }

    /// Register every shipped built-in walk: company names ending in a legal
    /// form, institutions around a head word, and a person's name after a
    /// title.
    #[must_use]
    pub fn with_builtin_walks(mut self) -> Self {
        self.walks.extend(shipped::walks::all());
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

    /// Set the [`RegexBudget`] for each *individual* variant regex.
    ///
    /// This is the knob to bound a caller-supplied rule: a legitimate single
    /// regex compiles well under 1 MiB, so a tight cap here rejects a
    /// pathological source at [`build`] without touching the (larger) union
    /// budget. A single budget cannot bound both, which is why the per-variant
    /// and [union](Self::with_union_budget) budgets are separate. [`build`]
    /// fails with a validation error when any one variant exceeds its budget.
    ///
    /// Any field left `None` leaves the `regex` crate's own default in place.
    ///
    /// [`build`]: Self::build
    #[must_use]
    pub fn with_variant_budget(mut self, budget: RegexBudget) -> Self {
        self.variant_budget = budget;
        self
    }

    /// Set the [`RegexBudget`] for the shared [`RegexSet`] union (every
    /// registered variant compiled together).
    ///
    /// Many individually-small regexes still union into one large `RegexSet`
    /// whose compiled size can only be bounded here; the shipped builtins
    /// routinely union past several MB, so keep this generous. [`build`] fails
    /// with a validation error when the union exceeds its budget.
    ///
    /// Any field left `None` leaves the `regex` crate's own default in place.
    ///
    /// [`build`]: Self::build
    #[must_use]
    pub fn with_union_budget(mut self, budget: RegexBudget) -> Self {
        self.union_budget = budget;
        self
    }

    /// Cap the total number of dictionary terms across **every** registered
    /// dictionary, they compile into one shared Aho-Corasick automaton, so
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

    /// Cap the total bytes of all dictionary terms across every dictionary,
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

    /// Set how context keywords are matched near a match.
    ///
    /// Defaults to [`SubstringMatcher`]. A [`LemmaMatcher`] also matches a
    /// keyword's other word forms when an enricher tokenized the text.
    ///
    /// [`LemmaMatcher`]: crate::context::matching::LemmaMatcher
    #[must_use]
    pub fn with_keyword_matcher(mut self, matcher: impl KeywordMatcher + 'static) -> Self {
        self.keyword_matcher = Some(Arc::new(matcher));
        self
    }

    /// Drop every pattern, dictionary, and walk none of whose candidate
    /// labels are declared in `catalog`.
    ///
    /// The engine uses this to build a per-request recognizer from a
    /// workspace-wide template, rules that could emit no label any policy
    /// declared never run. A rule is kept when *any* of its candidates is in
    /// the catalog; at recognize time it emits the first one that is. A
    /// dropped dictionary that a kept rule's `[context]` names still supplies
    /// that rule's keywords, without recognizing anything itself.
    #[must_use]
    pub fn filter_by_catalog(mut self, catalog: &LabelCatalog) -> Self {
        self.patterns
            .retain(|p| p.labels.iter().any(|l| catalog.contains(l)));
        self.walks
            .retain(|w| w.labels.iter().any(|l| catalog.contains(l)));
        let (kept, dropped): (Vec<_>, Vec<_>) = mem::take(&mut self.dictionaries)
            .into_iter()
            .partition(|d| d.labels.iter().any(|l| catalog.contains(l)));
        self.dictionaries = kept;

        let contexts = self
            .patterns
            .iter()
            .map(|p| &p.context)
            .chain(self.dictionaries.iter().map(|d| &d.context))
            .chain(self.walks.iter().map(|w| &w.context));
        let referenced: HashSet<&str> = contexts
            .flat_map(|c| c.dictionaries.iter().map(String::as_str))
            .collect();
        let sources: Vec<Dictionary> = mem::take(&mut self.context_sources)
            .into_iter()
            .chain(dropped)
            .filter(|d| referenced.contains(d.name.as_str()))
            .collect();
        self.context_sources = sources;
        self
    }

    /// Return `true` when no patterns, dictionaries, or walks are
    /// registered.
    ///
    /// The engine uses this to skip the per-request recognizer
    /// entirely after a catalog filter dropped every rule.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty() && self.dictionaries.is_empty() && self.walks.is_empty()
    }

    /// Borrow the accumulated patterns.
    #[must_use]
    pub fn patterns(&self) -> &[Pattern] {
        &self.patterns
    }

    /// Borrow the accumulated dictionaries.
    #[must_use]
    pub fn dictionaries(&self) -> &[Dictionary] {
        &self.dictionaries
    }

    /// Borrow the accumulated walks.
    #[must_use]
    pub fn walks(&self) -> &[Walk] {
        &self.walks
    }

    /// Compile every rule, and its `[context]`, into the pooled scanners.
    ///
    /// # Errors
    ///
    /// Returns a validation error when a pattern variant's regex
    /// fails to compile, when a variant references an unknown
    /// validator name, when a dictionary's `scoring` is invalid
    /// or under-declared for some term's source column, or when
    /// a walk's or the shared automata cannot be constructed.
    pub fn build(self) -> Result<PatternRecognizer> {
        let validators = self
            .validators
            .clone()
            .unwrap_or_else(ValidatorRegistry::builtin);
        let mut contexts = Vec::new();
        let (compiled_patterns, regex_set) = self.compile_patterns(&validators, &mut contexts)?;
        let (compiled_dicts, aho) = self.compile_dictionaries(&mut contexts)?;
        let walks = self
            .walks
            .iter()
            .map(|walk| {
                CompiledWalk::compile(walk, self.compile_context(&walk.context, &mut contexts))
            })
            .collect::<Result<_>>()?;

        Ok(PatternRecognizer {
            patterns: compiled_patterns,
            regex_set,
            dictionaries: compiled_dicts,
            aho,
            walks,
            contexts,
            keyword_matcher: self
                .keyword_matcher
                .unwrap_or_else(|| Arc::new(SubstringMatcher)),
        })
    }

    /// Compile `declared` into `contexts`, returning its index there, or
    /// `None` when it declares nothing.
    fn compile_context(
        &self,
        declared: &RuleContext,
        contexts: &mut Vec<CompiledContext>,
    ) -> Option<usize> {
        let sources = self.dictionaries.iter().chain(&self.context_sources);
        let context = CompiledContext::compile(declared, sources)?;
        contexts.push(context);
        Some(contexts.len() - 1)
    }

    /// Compile every `(pattern, variant)` pair into a
    /// [`CompiledPattern`] keyed by its slot in the shared
    /// [`RegexSet`].
    fn compile_patterns(
        &self,
        validators: &ValidatorRegistry,
        contexts: &mut Vec<CompiledContext>,
    ) -> Result<(Vec<CompiledPattern>, Option<RegexSet>)> {
        let variant_total: usize = self.patterns.iter().map(|p| p.variants.len()).sum();
        let mut compiled = Vec::with_capacity(variant_total);
        let mut regex_sources = Vec::with_capacity(variant_total);

        for pattern in &self.patterns {
            let context = self.compile_context(&pattern.context, contexts);
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
                let span_group = regex
                    .capture_names()
                    .position(|name| name == Some(Variant::ENTITY_GROUP));
                regex_sources.push(variant.regex.clone());
                compiled.push(CompiledPattern {
                    name: pattern.name.clone(),
                    labels: pattern.labels.clone(),
                    regex,
                    span_group,
                    score: variant.score,
                    validator,
                    languages: pattern.languages.clone(),
                    countries: pattern.countries.clone(),
                    context,
                });
            }
        }

        let regex_set = if regex_sources.is_empty() {
            None
        } else {
            let mut builder = RegexSetBuilder::new(&regex_sources);
            if let Some(bytes) = self.union_budget.nfa_size {
                builder.size_limit(bytes);
            }
            if let Some(bytes) = self.union_budget.dfa_size {
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

    /// Compile one regex source under the per-[`variant`](Self::with_variant_budget)
    /// budget. Unset budget fields leave the `regex` crate defaults untouched,
    /// so an unconfigured builder compiles exactly as before.
    fn compile_regex(&self, source: &str) -> StdResult<Regex, RegexError> {
        // A single variant is bounded by the per-variant budget, kept separate
        // from the (necessarily larger) shared-`RegexSet` union budget so a
        // tight per-rule cap does not break the builtin union.
        let mut builder = RegexBuilder::new(source);
        if let Some(bytes) = self.variant_budget.nfa_size {
            builder.size_limit(bytes);
        }
        if let Some(bytes) = self.variant_budget.dfa_size {
            builder.dfa_size_limit(bytes);
        }
        builder.build()
    }

    /// Compile every dictionary into a [`CompiledDictionary`]
    /// with its term-id range inside the shared Aho-Corasick
    /// automaton, plus per-term confidences resolved from the
    /// dictionary's `scoring` policy (with per-term overrides
    /// taking precedence).
    fn compile_dictionaries(
        &self,
        contexts: &mut Vec<CompiledContext>,
    ) -> Result<(Vec<CompiledDictionary>, Option<AhoCorasick>)> {
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
                // column didn't map to a declared score ,
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
                context: self.compile_context(&dict.context, contexts),
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
}
