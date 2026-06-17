//! [`RecognizerInput<M>`]: per-call input for a [`Recognizer`].
//!
//! Flat per-call surface for recognizers: the modality payload plus the
//! per-call concerns recognizers actually use — language hints,
//! candidate-language whitelist, jurisdiction hint, document-level
//! labels, out-of-band context strings, and a correlation id.
//!
//! [`Recognizer`]: super::Recognizer

use uuid::Uuid;

use crate::modality::Modality;
use crate::primitive::{CountryCode, LanguageTag};

/// Per-call input for a [`Recognizer`](super::Recognizer).
///
/// Bundles the modality payload ([`content`](Self::content), the
/// modality's span/data) with the per-call concerns recognizers
/// actually use.
#[derive(Debug)]
pub struct RecognizerInput<M: Modality> {
    /// The modality payload to inspect, in modality-local coordinates.
    pub content: M::Data,
    /// Caller-asserted language. When `Some`, recognizers that support
    /// per-call language hinting (typically NER / LLM backends) skip
    /// their own detection.
    pub language: Option<LanguageTag>,
    /// Restrict language auto-detection to this subset when
    /// [`language`](Self::language) is `None`. Empty means "any".
    pub candidate_languages: Vec<LanguageTag>,
    /// Caller-asserted jurisdiction. When `Some`, recognizers that
    /// carry per-rule country scopes skip rules that don't match.
    /// `None` means "any" — rules that declare countries still run as a
    /// permissive fallback so callers who don't pass a hint don't lose
    /// detections.
    pub country: Option<CountryCode>,
    /// Document-level classification labels (e.g. `"medical"`,
    /// `"gdpr-request"`). Recognizers may use these to bias their
    /// behavior for domain-specific terms; those that don't ignore the
    /// field.
    pub labels: Vec<String>,
    /// Out-of-band context strings the caller wants treated as
    /// in-context for confidence boosting (e.g. the column header of a
    /// CSV cell, the JSON object key of a string value, the log field
    /// name a value sits under). Recognizers that run a context
    /// enhancer feed these to the enhancer alongside the in-text word
    /// window; recognizers without an enhancer ignore the field.
    pub context_hints: Vec<String>,
    /// Correlation UUID propagated through the tracing span for this
    /// call. Recognizer bodies do not read this directly; it's set on
    /// the span by the caller.
    pub correlation_id: Option<Uuid>,
}

impl<M: Modality> RecognizerInput<M> {
    /// Construct an input with only the modality payload set; every
    /// other field defaults to empty.
    pub fn new(content: M::Data) -> Self {
        Self {
            content,
            language: None,
            candidate_languages: Vec::new(),
            country: None,
            labels: Vec::new(),
            context_hints: Vec::new(),
            correlation_id: None,
        }
    }

    /// Set the asserted language.
    #[must_use]
    pub fn with_language(mut self, language: LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Set the candidate languages for auto-detection.
    #[must_use]
    pub fn with_candidate_languages(mut self, languages: Vec<LanguageTag>) -> Self {
        self.candidate_languages = languages;
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

    /// Attach out-of-band context hint strings (column headers, JSON
    /// keys, …) the enhancer should treat as in-context.
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

    /// Whether a recognizer rule scoped to `allowed` languages should
    /// run for this call.
    ///
    /// - An empty `allowed` list means the rule is language-agnostic
    ///   and always runs.
    /// - When `allowed` is non-empty and [`language`](Self::language) is
    ///   `Some(_)`, the rule runs when the hint shares a primary subtag
    ///   with any entry in `allowed` (so an `["en"]` rule fires for
    ///   `"en-US"` and `"en-GB"` hints).
    /// - When [`language`](Self::language) is `None`, the rule still
    ///   runs — we can't disprove applicability without a hint.
    #[must_use]
    pub fn applies_to_language(&self, allowed: &[LanguageTag]) -> bool {
        if allowed.is_empty() {
            return true;
        }
        match self.language.as_ref() {
            Some(hint) => allowed.iter().any(|a| a.matches(hint)),
            None => true,
        }
    }

    /// Whether a recognizer rule scoped to `allowed` countries should
    /// run for this call.
    ///
    /// - An empty `allowed` list means the rule is jurisdiction-agnostic
    ///   and always runs.
    /// - When `allowed` is non-empty and [`country`](Self::country) is
    ///   `Some(_)`, the rule runs only when the hint is in `allowed`.
    /// - When [`country`](Self::country) is `None`, the rule still runs
    ///   — we can't disprove applicability without a hint, and silently
    ///   dropping detections would surprise callers who simply forgot to
    ///   set the field.
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
