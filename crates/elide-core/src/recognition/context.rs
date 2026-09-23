//! [`RecognizerContext<M>`]: the per-payload view a [`Recognizer`] sees.
//!
//! [`Recognizer`]: super::Recognizer

use hipstr::HipStr;
use uuid::Uuid;

use crate::entity::{Label, LabelCatalog, LabelRef};
use crate::modality::Modality;
use crate::primitive::CountryCode;
use crate::recognition::annotation::{Annotations, Exclusion, Inclusion};
use crate::recognition::{Languages, Scope, Subject};

/// Analysis-wide context handed to a [`Recognizer`] alongside the [`Subject`].
///
/// Borrows the caller-asserted [`Scope`] (shared across every chunk of the
/// analysis) and the per-modality region [`Annotations`]. Where the [`Subject`]
/// carries the per-chunk working state (payload, enrichment, detected
/// languages, hints), this carries what is fixed for the whole run: the target
/// labels, jurisdictions, tags, inclusions, and exclusions.
///
/// Query the call's languages, jurisdictions, labels, inclusions, and
/// exclusions through the methods here rather than reaching into the scope
/// directly: the language methods fold the caller's assertions together with
/// what a detector found on the [`Subject`].
///
/// [`Recognizer`]: super::Recognizer
/// [`Scope`]: super::Scope
/// [`Annotations`]: super::annotation::Annotations
#[derive(Debug)]
pub struct RecognizerContext<'a, M: Modality> {
    /// Caller-asserted, modality-free scope for the analysis (shared,
    /// immutable).
    scope: &'a Scope,
    /// Caller-supplied per-modality region annotations (inclusions /
    /// exclusions). `None` (the default) means none asserted, read as empty
    /// slices by [`inclusions`] / [`exclusions`].
    ///
    /// [`inclusions`]: Self::inclusions
    /// [`exclusions`]: Self::exclusions
    annotations: Option<&'a Annotations<M>>,
}

impl<'a, M: Modality> RecognizerContext<'a, M> {
    /// Context over `scope` with no region annotations. Attach annotations with
    /// [`with_annotations`].
    ///
    /// [`with_annotations`]: Self::with_annotations
    #[must_use]
    pub fn new(scope: &'a Scope) -> Self {
        Self {
            scope,
            annotations: None,
        }
    }

    /// Attach the caller's per-modality [`Annotations`] (inclusion /
    /// exclusion regions) for this analysis.
    ///
    /// [`Annotations`]: super::annotation::Annotations
    #[must_use]
    pub fn with_annotations(mut self, annotations: &'a Annotations<M>) -> Self {
        self.annotations = Some(annotations);
        self
    }

    /// Caller-asserted [`Scope`] this context borrows.
    ///
    /// [`Scope`]: super::Scope
    #[must_use]
    pub fn scope(&self) -> &Scope {
        self.scope
    }

    /// Caller-supplied [`Inclusion`] regions for this analysis, or an empty
    /// slice when none were asserted.
    #[must_use]
    pub fn inclusions(&self) -> &[Inclusion<M>] {
        self.annotations.map_or(&[], |a| &a.inclusions)
    }

    /// Caller-supplied [`Exclusion`] regions for this analysis, or an empty
    /// slice when none were asserted.
    #[must_use]
    pub fn exclusions(&self) -> &[Exclusion<M>] {
        self.annotations.map_or(&[], |a| &a.exclusions)
    }

    /// Caller-asserted document-level classification tags for this
    /// analysis (e.g. `"medical"`). Distinct from the entity types to emit,
    /// those are [`target_labels`].
    ///
    /// [`target_labels`]: Self::target_labels
    #[must_use]
    pub fn tags(&self) -> &[HipStr<'static>] {
        &self.scope.metadata.tags
    }

    /// The caller-asserted business purpose driving this request (e.g.
    /// `"fraud_detection"`), or `None` if unasserted. A recognizer may bias
    /// detection on it.
    #[must_use]
    pub fn purpose(&self) -> Option<&HipStr<'static>> {
        self.scope.metadata.purpose.as_ref()
    }

    /// Who the redacted output is for (e.g. `"auditor"`). A recognizer may
    /// bias detection on it.
    #[must_use]
    pub fn audience(&self) -> &[HipStr<'static>] {
        &self.scope.metadata.audience
    }

    /// The [`LabelCatalog`] of entity types to detect, the caller's request.
    /// A zero-shot NER model requests exactly these labels; an LLM prompt lists
    /// them as the types to find. On the `Analyzer::analyze*` path this is
    /// non-empty: those entries gate an empty catalog (detect nothing) before
    /// any recognizer runs. A recognizer driven directly still handles an empty
    /// catalog as its own labels dictate.
    #[must_use]
    pub fn catalog(&self) -> &LabelCatalog {
        &self.scope.catalog
    }

    /// The entity types to emit, as [`LabelRef`]s, the catalog's labels.
    /// Convenience over [`catalog`] for recognizers that
    /// only need the ids.
    ///
    /// [`catalog`]: Self::catalog
    #[must_use]
    pub fn target_labels(&self) -> Vec<LabelRef> {
        self.scope.catalog.refs().collect()
    }

    /// The entity types to emit, as full [`Label`]s, so a prompt or backend
    /// can render each label's localized display name and description in the
    /// call's primary language (English fallback) while still keying on the
    /// stable id. The catalog's labels, cloned.
    #[must_use]
    pub fn target_label_defs(&self) -> Vec<Label> {
        self.scope.catalog.iter().cloned().collect()
    }

    /// Correlation id, if the caller set one.
    #[must_use]
    pub fn correlation_id(&self) -> Option<Uuid> {
        self.scope.correlation_id
    }

    /// Whether the caller asserted any language on the scope.
    ///
    /// An enricher consults this to decide whether to run detection: a
    /// caller assertion is authoritative, so detection can be skipped.
    #[must_use]
    pub fn has_asserted_language(&self) -> bool {
        !self.scope.languages.is_empty()
    }

    /// The call's languages — the caller-asserted ones (scope) and the ones a
    /// detector found (on `subject`) — as a [`Languages`] view to query
    /// together: which is primary, whether a language-scoped rule applies, and
    /// span-based entity attribution.
    #[must_use]
    pub fn languages<'s>(&'s self, subject: &'s Subject<M>) -> Languages<'s> {
        Languages::new(
            self.scope.languages.as_slice(),
            subject.detected_languages(),
        )
    }

    /// Whether a recognizer rule scoped to `allowed` countries should run
    /// for this call.
    ///
    /// - An empty `allowed` list means the rule is jurisdiction-agnostic
    ///   and always runs.
    /// - When `allowed` is non-empty and the scope asserts countries, the
    ///   rule runs when any asserted country is in `allowed`.
    /// - When the scope asserts no countries, the rule still runs: we
    ///   can't disprove applicability without an assertion.
    #[must_use]
    pub fn applies_to_country(&self, allowed: &[CountryCode]) -> bool {
        if allowed.is_empty() || self.scope.countries.is_empty() {
            return true;
        }
        self.scope.countries.iter().any(|c| allowed.contains(c))
    }
}
