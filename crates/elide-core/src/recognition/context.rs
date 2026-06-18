//! [`RecognizerContext<M>`]: per-call context for a [`Recognizer`].
//!
//! Flat per-call surface of the concerns recognizers consult alongside
//! the payload: the call's languages, asserted jurisdictions,
//! document-level labels, out-of-band context strings, shared NLP
//! artifacts, caller hints, and a correlation id. The modality payload
//! is passed separately to [`Recognizer::recognize`]; the context is
//! built once and reused across every payload of one analysis.
//!
//! [`Recognizer`]: super::Recognizer
//! [`Recognizer::recognize`]: super::Recognizer::recognize

use uuid::Uuid;

use crate::modality::Modality;
use crate::primitive::{Confidence, CountryCode, Language, LanguageTag, Languages};
use crate::recognition::{Artifacts, Hint};

/// Per-call context for a [`Recognizer`].
///
/// Holds the per-call concerns recognizers consult; the modality payload
/// is passed alongside it to [`Recognizer::recognize`]. Built once with
/// the `with_*` chain and reused across every payload of one analysis.
///
/// [`Recognizer`]: super::Recognizer
/// [`Recognizer::recognize`]: super::Recognizer::recognize
#[derive(Debug)]
pub struct RecognizerContext<M: Modality> {
    /// Shared per-call NLP enrichment (tokens, lemmas, …), keyed by type.
    /// An enricher computes it once; recognizers that want it read it back
    /// by type. Those that don't leave it empty.
    pub artifacts: Artifacts,
    /// The call's languages: each entry is a language with how it was
    /// obtained (detected by an enricher, or asserted by the caller), an
    /// optional confidence, and an optional span. Empty means "unknown".
    /// Consult it through the `RecognizerLanguage` trait rather than
    /// indexing directly.
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
    /// Out-of-band context strings the caller wants treated as in-context
    /// for confidence boosting (e.g. the column header of a CSV cell, the
    /// JSON object key of a string value, the log field name a value sits
    /// under). Recognizers that run a context enhancer feed these to the
    /// enhancer alongside the in-text word window; recognizers without an
    /// enhancer ignore the field.
    pub context_hints: Vec<String>,
    /// Caller-supplied annotation regions (a region the caller believes
    /// may hold an entity, with an optional claimed label and name).
    /// Recognizers that adjudicate hints (typically LLM-based) fold these
    /// into detection to confirm, relocate, or reject each one; the rest
    /// ignore the field.
    pub hints: Vec<Hint<M>>,
    /// Correlation UUID propagated through the tracing span for this call.
    /// Recognizer bodies do not read this directly; it's set on the span
    /// by the caller.
    pub correlation_id: Option<Uuid>,
}

impl<M: Modality> RecognizerContext<M> {
    /// An empty context: nothing asserted, every field defaults to empty.
    pub fn new() -> Self {
        Self {
            artifacts: Artifacts::new(),
            languages: Languages::default(),
            countries: Vec::new(),
            labels: Vec::new(),
            context_hints: Vec::new(),
            hints: Vec::new(),
            correlation_id: None,
        }
    }

    /// Replace the artifacts bundle.
    #[must_use]
    pub fn with_artifacts(mut self, artifacts: Artifacts) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// A fresh per-call context carrying this one's caller-asserted fields
    /// (languages, countries, labels, hints, correlation id) but an empty
    /// [`Artifacts`] bundle.
    ///
    /// The caller builds the asserted context once; the analyzer forks a
    /// working copy per payload so each call's enrichers stamp their own
    /// artifacts and detected languages without sharing across payloads.
    /// [`Artifacts`] is per-call NLP output, so it is never carried over.
    #[must_use]
    pub fn fork_assertions(&self) -> Self {
        Self {
            artifacts: Artifacts::new(),
            languages: self.languages.clone(),
            countries: self.countries.clone(),
            labels: self.labels.clone(),
            context_hints: self.context_hints.clone(),
            hints: self.hints.clone(),
            correlation_id: self.correlation_id,
        }
    }

    /// Attach caller-supplied annotation [`Hint`]s.
    #[must_use]
    pub fn with_hints(mut self, hints: Vec<Hint<M>>) -> Self {
        self.hints = hints;
        self
    }

    /// Assert a language for this call, returning `self` for chaining.
    ///
    /// Adds a caller-asserted [`Language`] to the call's
    /// languages. `confidence` is optional; an assertion outranks a
    /// detection at equal confidence (see [`RecognizerLanguage::languages`]).
    ///
    /// [`RecognizerLanguage::languages`]: super::RecognizerLanguage::languages
    #[must_use]
    pub fn with_language(mut self, language: LanguageTag, confidence: Option<Confidence>) -> Self {
        self.languages
            .push(Language::asserted(language, confidence));
        self
    }

    /// Assert a jurisdiction for this call. May be called more than once
    /// to assert several; a rule runs when any one matches.
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

    /// Attach out-of-band context hint strings (column headers, JSON keys,
    /// …) the enhancer should treat as in-context.
    #[must_use]
    pub fn with_context_hints(mut self, hints: Vec<String>) -> Self {
        self.context_hints = hints;
        self
    }

    /// Set the correlation id propagated through the tracing span.
    #[must_use]
    pub fn with_correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Whether a recognizer rule scoped to `allowed` countries should run
    /// for this call.
    ///
    /// - An empty `allowed` list means the rule is jurisdiction-agnostic
    ///   and always runs.
    /// - When `allowed` is non-empty and [`countries`] is non-empty, the
    ///   rule runs when any asserted country is in `allowed`.
    /// - When [`countries`] is empty, the rule still runs: we can't
    ///   disprove applicability without an assertion, and silently
    ///   dropping detections would surprise callers who simply forgot to
    ///   set the field.
    ///
    /// [`countries`]: Self::countries
    #[must_use]
    pub fn applies_to_country(&self, allowed: &[CountryCode]) -> bool {
        if allowed.is_empty() || self.countries.is_empty() {
            return true;
        }
        self.countries.iter().any(|c| allowed.contains(c))
    }
}

impl<M: Modality> Default for RecognizerContext<M> {
    fn default() -> Self {
        Self::new()
    }
}
