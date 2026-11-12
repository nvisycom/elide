//! [`Context`]: the analysis-wide inputs an [`Enricher`] and [`Recognizer`]
//! read alongside each [`Subject`].
//!
//! [`Enricher`]: crate::enrichment::Enricher
//! [`Recognizer`]: super::Recognizer
//! [`Subject`]: super::Subject

use hipstr::HipStr;
use uuid::Uuid;

use crate::entity::{Label, LabelCatalog, LabelRef};
use crate::modality::Modality;
use crate::primitive::CountryCode;
use crate::recognition::{Languages, Scope, Subject};

/// The analysis-wide inputs an enricher or recognizer gets with a [`Subject`].
///
/// Borrows the caller-asserted [`Scope`], shared across every chunk of the
/// analysis and every modality. Where the [`Subject`] carries the per-chunk
/// working state (payload, enrichment, detected languages, hints), this carries
/// what the caller fixed for the whole run: the target labels, jurisdictions,
/// and tags. Both enrichers and recognizers read it, hence a neutral name, not
/// one tied to either.
///
/// Query the call's languages, jurisdictions, and labels through the methods
/// here rather than reaching into the scope directly: the language methods fold
/// the caller's assertions together with what a detector found on the
/// [`Subject`].
///
/// [`Scope`]: super::Scope
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// Caller-asserted, modality-free scope for the analysis (shared,
    /// immutable).
    scope: &'a Scope,
}

impl<'a> Context<'a> {
    /// Context over `scope`.
    #[must_use]
    pub fn new(scope: &'a Scope) -> Self {
        Self { scope }
    }

    /// Caller-asserted [`Scope`] this context borrows.
    ///
    /// [`Scope`]: super::Scope
    #[must_use]
    pub fn scope(&self) -> &Scope {
        self.scope
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

    /// The [`LabelCatalog`] of entity types to detect, as the caller requested.
    ///
    /// A zero-shot NER model requests exactly these labels; an LLM prompt lists
    /// them as the types to find. On the `Analyzer::analyze*` path this is
    /// non-empty: those entries gate an empty catalog (detect nothing) before
    /// any recognizer runs. A recognizer driven directly still handles an empty
    /// catalog as its own labels dictate.
    #[must_use]
    pub fn catalog(&self) -> &LabelCatalog {
        &self.scope.catalog
    }

    /// The entity types to emit, as the catalog's [`LabelRef`]s.
    ///
    /// Convenience over [`catalog`] for recognizers that only need the ids.
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

    /// The call's languages, asserted and detected, as a [`Languages`] view.
    ///
    /// The caller-asserted ones (scope) and the ones a detector found (on
    /// `subject`), to query together: which is primary, whether a
    /// language-scoped rule applies, and span-based entity attribution.
    #[must_use]
    pub fn languages<'s, M: Modality>(&'s self, subject: &'s Subject<M>) -> Languages<'s> {
        Languages::new(self.scope.languages(), subject.detected_languages())
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
