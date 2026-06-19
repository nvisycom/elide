//! [`Scope<M>`]: the caller-asserted scope of one analysis.

use uuid::Uuid;

use crate::modality::Modality;
use crate::primitive::{CountryCode, Language, Languages};
use crate::recognition::annotation::{Exclusion, Inclusion};

/// Caller-asserted scope shared across every payload of one analysis.
///
/// Built once with the `with_*` chain and passed by reference to the
/// analyzer, which borrows it into a fresh [`RecognizerContext`] per
/// payload. It holds only what the *caller* asserts (languages,
/// jurisdictions, document labels, inclusion and exclusion regions, a
/// correlation id); the per-payload working state (NLP artifacts,
/// detected languages) lives on the context, not here.
///
/// [`RecognizerContext`]: super::RecognizerContext
#[derive(Debug)]
pub struct Scope<M: Modality> {
    /// Caller-asserted languages for the analysis. Empty means the caller
    /// asserted none, leaving detection (if an enricher runs) to fill in.
    pub languages: Languages,
    /// Caller-asserted jurisdictions. When non-empty, recognizers that
    /// carry per-rule country scopes skip rules that match none of them.
    /// An empty list means "any": rules that declare countries still run
    /// as a permissive fallback so callers who don't assert a jurisdiction
    /// don't lose detections. A document spanning several jurisdictions
    /// can assert all of them; a rule runs when any one matches.
    pub countries: Vec<CountryCode>,
    /// Document-level classification labels (e.g. `"medical"`,
    /// `"gdpr-request"`). Recognizers may use these to bias their behavior
    /// for domain-specific terms; those that don't ignore the field.
    pub labels: Vec<String>,
    /// Caller-supplied candidate regions (each a region the caller
    /// believes may hold an entity, with an optional claimed label, name,
    /// and confidence). Recognizers that adjudicate inclusions (typically
    /// LLM-based) fold these into detection to confirm, relocate, or
    /// reject each one; the rest ignore the field.
    pub inclusions: Vec<Inclusion<M>>,
    /// Caller-supplied protected regions. The analyzer drops any entity
    /// whose location overlaps an exclusion, regardless of which
    /// recognizer found it.
    pub exclusions: Vec<Exclusion<M>>,
    /// Correlation UUID propagated through the tracing span for this
    /// analysis.
    pub correlation_id: Option<Uuid>,
}

impl<M: Modality> Scope<M> {
    /// Empty scope: nothing asserted.
    pub fn new() -> Self {
        Self {
            languages: Languages::default(),
            countries: Vec::new(),
            labels: Vec::new(),
            inclusions: Vec::new(),
            exclusions: Vec::new(),
            correlation_id: None,
        }
    }

    /// Assert a language for the analysis, returning `self` for chaining.
    ///
    /// Build the [`Language`] with [`Language::asserted`] (optionally
    /// [`with_confidence`]); an assertion outranks a detection at equal
    /// confidence.
    ///
    /// [`with_confidence`]: Language::with_confidence
    #[must_use]
    pub fn with_language(mut self, language: Language) -> Self {
        self.languages.push(language);
        self
    }

    /// Assert a jurisdiction. May be called more than once to assert
    /// several; a rule runs when any one matches.
    #[must_use]
    pub fn with_country(mut self, country: CountryCode) -> Self {
        self.countries.push(country);
        self
    }

    /// Replace the asserted jurisdictions with `countries`.
    #[must_use]
    pub fn with_countries(mut self, countries: Vec<CountryCode>) -> Self {
        self.countries = countries;
        self
    }

    /// Attach document-level classification labels.
    #[must_use]
    pub fn with_labels(mut self, labels: Vec<String>) -> Self {
        self.labels = labels;
        self
    }

    /// Attach caller-supplied [`Inclusion`] regions.
    #[must_use]
    pub fn with_inclusions(mut self, inclusions: Vec<Inclusion<M>>) -> Self {
        self.inclusions = inclusions;
        self
    }

    /// Attach caller-supplied [`Exclusion`] regions.
    #[must_use]
    pub fn with_exclusions(mut self, exclusions: Vec<Exclusion<M>>) -> Self {
        self.exclusions = exclusions;
        self
    }

    /// Set the correlation id propagated through the tracing span.
    #[must_use]
    pub fn with_correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }
}

impl<M: Modality> Default for Scope<M> {
    fn default() -> Self {
        Self::new()
    }
}
