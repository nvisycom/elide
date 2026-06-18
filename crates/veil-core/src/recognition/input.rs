//! [`RecognizerInput<M>`]: per-call input for a [`Recognizer`].
//!
//! Flat per-call surface for recognizers: the modality payload plus the
//! per-call concerns recognizers actually use (the call's languages, a
//! jurisdiction hint, document-level labels, out-of-band context strings,
//! shared NLP artifacts, and a correlation id).
//!
//! [`Recognizer`]: super::Recognizer

use uuid::Uuid;

use crate::modality::Modality;
use crate::primitive::{
    Confidence, CountryCode, LanguageDetection, LanguageDetections, LanguageTag,
};
use crate::recognition::{Artifacts, Hint};

/// Per-call input for a [`Recognizer`].
///
/// Bundles the modality payload ([`content`], the modality's span/data)
/// with the per-call concerns recognizers actually use.
///
/// [`Recognizer`]: super::Recognizer
/// [`content`]: Self::content
#[derive(Debug)]
pub struct RecognizerInput<M: Modality> {
    /// The modality payload to inspect, in modality-local coordinates.
    pub content: M::Data,
    /// Shared per-call NLP enrichment (tokens, lemmas, …), keyed by type.
    /// An enricher computes it once; recognizers that want it read it back
    /// by type. Those that don't leave it empty.
    pub artifacts: Artifacts,
    /// The call's languages: each entry is a language with how it was
    /// obtained (detected by an enricher, or asserted by the caller), an
    /// optional confidence, and an optional span. Empty means "unknown".
    /// Consult it through the `RecognizerLanguage` trait rather than
    /// indexing directly.
    pub languages: LanguageDetections,
    /// Caller-asserted jurisdiction. When `Some`, recognizers that carry
    /// per-rule country scopes skip rules that don't match. `None` means
    /// "any": rules that declare countries still run as a permissive
    /// fallback so callers who don't pass a hint don't lose detections.
    pub country: Option<CountryCode>,
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

impl<M: Modality> RecognizerInput<M> {
    /// Construct an input with only the modality payload set; every other
    /// field defaults to empty.
    pub fn new(content: M::Data) -> Self {
        Self {
            content,
            artifacts: Artifacts::new(),
            languages: LanguageDetections::default(),
            country: None,
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

    /// Attach caller-supplied annotation [`Hint`]s.
    #[must_use]
    pub fn with_hints(mut self, hints: Vec<Hint<M>>) -> Self {
        self.hints = hints;
        self
    }

    /// Assert a language for this call, returning `self` for chaining.
    ///
    /// Adds a caller-asserted [`LanguageDetection`] to the call's
    /// languages. `confidence` is optional; an assertion outranks a
    /// detection at equal confidence (see [`RecognizerLanguage::languages`]).
    ///
    /// [`RecognizerLanguage::languages`]: super::RecognizerLanguage::languages
    #[must_use]
    pub fn with_language(mut self, language: LanguageTag, confidence: Option<Confidence>) -> Self {
        self.languages
            .push(LanguageDetection::asserted(language, confidence));
        self
    }

    /// Set the asserted jurisdiction.
    #[must_use]
    pub fn with_country(mut self, country: CountryCode) -> Self {
        self.country = Some(country);
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
    /// - When `allowed` is non-empty and [`country`] is `Some(_)`, the
    ///   rule runs only when the hint is in `allowed`.
    /// - When [`country`] is `None`, the rule still runs: we can't
    ///   disprove applicability without a hint, and silently dropping
    ///   detections would surprise callers who simply forgot to set the
    ///   field.
    ///
    /// [`country`]: Self::country
    #[must_use]
    pub fn applies_to_country(&self, allowed: &[CountryCode]) -> bool {
        if allowed.is_empty() {
            return true;
        }
        match self.country.as_ref() {
            Some(hint) => allowed.iter().any(|a| a == hint),
            None => true,
        }
    }
}
