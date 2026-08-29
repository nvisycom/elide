//! [`Scope`]: the caller-asserted, modality-independent scope of one
//! analysis, and its free-form [`ScopeMetadata`].

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::LabelCatalog;
use crate::primitive::{CountryCode, Language, Languages};

/// Caller-asserted scope shared across every payload of one analysis.
///
/// Built once with the `with_*` chain and passed by reference to the
/// analyzer, which borrows it into a fresh [`RecognizerContext`] per
/// payload. It holds only what the *caller* asserts about the analysis as a
/// whole — languages, jurisdictions, document labels, the target catalog, a
/// correlation id — none of which depends on the medium, so one [`Scope`]
/// drives a text, image, or audio analysis alike.
///
/// Per-medium regions (caller-supplied inclusions and exclusions, which are
/// `M::Location`-typed) live in [`Annotations`], attached to the analyzer
/// of that modality. The per-payload working state (NLP artifacts, detected
/// languages) lives on the context, not here.
///
/// [`RecognizerContext`]: super::RecognizerContext
/// [`Annotations`]: super::annotation::Annotations
/// Free-form, caller-asserted request context: the *document* it is about and
/// the *request* driving it.
///
/// Three axes of opaque classification strings elide neither ships nor
/// interprets — a downstream policy layer chooses what `"medical"` or
/// `"fraud_detection"` or `"auditor"` mean. They are read in two places: a
/// recognizer may bias its detection on them (the LLM prompt lists them so the
/// model attends to the right terms), and a scope-aware operator predicate may
/// branch on them at selection time (redact the same document differently per
/// [`audience`]).
///
/// - [`tags`] classify the *document* (`"medical"`, `"gdpr-request"`).
/// - [`purpose`] is why the request exists (`"fraud_detection"`).
/// - [`audience`] is who the redacted output is for (`"support_agent"`,
///   `"auditor"`) — the axis PCI-style "same document, two masks" branches on.
///
/// [`tags`]: Self::tags
/// [`purpose`]: Self::purpose
/// [`audience`]: Self::audience
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ScopeMetadata {
    /// Document-level classification tags (e.g. `"medical"`,
    /// `"gdpr-request"`). Recognizers may use these to bias their behavior
    /// for domain-specific terms; those that don't ignore the field.
    ///
    /// Named `tags`, not `labels`, to keep "label" reserved for the entity
    /// taxonomy ([`LabelRef`]/[`LabelCatalog`]): these classify the
    /// *document*, whereas the scope's catalog names the entity *types* to
    /// emit.
    ///
    /// [`LabelRef`]: crate::entity::LabelRef
    /// [`LabelCatalog`]: crate::entity::LabelCatalog
    #[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
    pub tags: Vec<HipStr<'static>>,
    /// The caller-asserted business purpose driving this request (e.g.
    /// `"fraud_detection"`, `"gdpr_erasure_request"`). A scope-aware operator
    /// predicate may skip or swap a rule based on it; a recognizer may bias
    /// detection on it. `None` when the caller asserts no purpose.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub purpose: Option<HipStr<'static>>,
    /// Who the redacted output is for (e.g. `"support_agent"`, `"auditor"`).
    /// The axis a per-audience redaction branches on: one detected document,
    /// selected differently per audience. May hold several.
    #[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
    pub audience: Vec<HipStr<'static>>,
}

impl ScopeMetadata {
    /// Whether the caller asserted no request metadata: no [`tags`], no
    /// [`purpose`], and no [`audience`]. Equal to [`ScopeMetadata::default`].
    ///
    /// [`tags`]: Self::tags
    /// [`purpose`]: Self::purpose
    /// [`audience`]: Self::audience
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.purpose.is_none() && self.audience.is_empty()
    }
}

#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Scope {
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
    /// Free-form request context — document tags, request purpose, and the
    /// output audience. See [`ScopeMetadata`].
    pub metadata: ScopeMetadata,
    /// The entity types to detect — the caller's request. A zero-shot NER
    /// model requests exactly this set; an LLM prompt lists it as the labels
    /// to find; every detection is culled to it. **Empty means detect
    /// nothing**: an empty catalog requests no types, so the analyzer
    /// short-circuits before any recognizer runs. Pass an explicit set, or
    /// [`LabelCatalog::with_builtins`], to detect anything. (A recognizer's own
    /// `supported_labels` still select a subset of a *non-empty* catalog.)
    pub catalog: LabelCatalog,
    /// Correlation UUID propagated through the tracing span for this
    /// analysis.
    pub correlation_id: Option<Uuid>,
}

impl Scope {
    /// Empty scope: nothing asserted. Its [`catalog`](Self::catalog) is empty,
    /// so a bare `Scope::new()` **detects nothing** — set a catalog (e.g.
    /// `.with_catalog(`[`LabelCatalog::with_builtins`]`())`) to detect.
    pub fn new() -> Self {
        Self {
            languages: Languages::default(),
            countries: Vec::new(),
            metadata: ScopeMetadata::default(),
            catalog: LabelCatalog::new(),
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

    /// Attach document-level classification [tags](ScopeMetadata::tags)
    /// (e.g. `"medical"`).
    #[must_use]
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<HipStr<'static>>>) -> Self {
        self.metadata.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Assert the business [purpose](ScopeMetadata::purpose) driving this
    /// request (e.g. `"fraud_detection"`).
    #[must_use]
    pub fn with_purpose(mut self, purpose: impl Into<HipStr<'static>>) -> Self {
        self.metadata.purpose = Some(purpose.into());
        self
    }

    /// Set the [audience](ScopeMetadata::audience) the redacted output is for
    /// (e.g. `"auditor"`) — the axis a per-audience redaction branches on.
    #[must_use]
    pub fn with_audience(
        mut self,
        audience: impl IntoIterator<Item = impl Into<HipStr<'static>>>,
    ) -> Self {
        self.metadata.audience = audience.into_iter().map(Into::into).collect();
        self
    }

    /// Replace the whole [`ScopeMetadata`] block at once.
    #[must_use]
    pub fn with_metadata(mut self, metadata: ScopeMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set the [`LabelCatalog`] of entity types to detect — the request.
    ///
    /// Threaded onto every [`RecognizerContext`]; a zero-shot NER model
    /// requests exactly these labels, an LLM prompt lists them as the types to
    /// find, and every detection is culled to this set. An empty catalog
    /// requests nothing, so the analyzer detects nothing —
    /// [`LabelCatalog::with_builtins`] is the ready-made "detect every built-in
    /// type" set. (A recognizer with its own `supported_labels` still selects a
    /// subset of a non-empty catalog.)
    ///
    /// [`RecognizerContext`]: super::RecognizerContext
    #[must_use]
    pub fn with_catalog(mut self, catalog: LabelCatalog) -> Self {
        self.catalog = catalog;
        self
    }

    /// Set the correlation id propagated through the tracing span.
    #[must_use]
    pub fn with_correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_empty_by_default_and_when_any_field_is_set() {
        assert!(ScopeMetadata::default().is_empty());

        assert!(!Scope::new().with_tags(["medical"]).metadata.is_empty());
        assert!(!Scope::new().with_purpose("fraud").metadata.is_empty());
        assert!(!Scope::new().with_audience(["auditor"]).metadata.is_empty());
    }
}
