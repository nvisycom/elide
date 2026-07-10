//! [`Directives`]: the caller's per-analysis inputs, passed to
//! [`Orchestrator::analyze`].
//!
//! [`Orchestrator::analyze`]: super::Orchestrator::analyze

use std::any::{Any, TypeId};
use std::collections::HashMap;

use elide_core::modality::Modality;
use elide_core::recognition::Scope;
use elide_core::recognition::annotation::Annotations;

/// Per-analysis region [`Annotations`], one set per modality.
///
/// A container document spans several modalities at once (a text body plus
/// image parts, …), each with its own `M::Location`-typed regions. The set
/// keys an erased [`Annotations<M>`] by the modality's [`TypeId`], so the
/// modality-erased [`analyze`] path recovers each pipeline's regions by its
/// own `M`.
///
/// [`Annotations`]: elide_core::recognition::annotation::Annotations
/// [`analyze`]: super::Orchestrator::analyze
#[derive(Default)]
pub(crate) struct AnnotationSet {
    by_modality: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl AnnotationSet {
    /// Attach the region annotations for modality `M`, replacing any already
    /// set for `M`.
    #[must_use]
    pub(crate) fn with<M: Modality>(mut self, annotations: Annotations<M>) -> Self {
        self.by_modality
            .insert(TypeId::of::<M>(), Box::new(annotations));
        self
    }

    /// The regions for `M`, or an empty [`Annotations`] when none were
    /// attached for that modality.
    ///
    /// [`Annotations`]: elide_core::recognition::annotation::Annotations
    pub(crate) fn get<M: Modality>(&self) -> Annotations<M> {
        self.by_modality
            .get(&TypeId::of::<M>())
            .and_then(|a| a.downcast_ref::<Annotations<M>>())
            .cloned()
            .unwrap_or_default()
    }
}

/// The caller's inputs for one [`analyze`] call: per-modality region
/// annotations plus an optional [`Scope`] override.
///
/// The annotations describe regions in this document. The scope, when set,
/// overrides the orchestrator's run-wide [`with_scope`] default for this call
/// only.
///
/// [`analyze`]: super::Orchestrator::analyze
/// [`with_scope`]: super::Orchestrator::with_scope
/// [`Scope`]: elide_core::recognition::Scope
#[derive(Default)]
pub struct Directives {
    pub(crate) annotations: AnnotationSet,
    pub(crate) scope: Option<Scope>,
}

impl Directives {
    /// Empty directives: no regions, and the orchestrator's run-wide scope.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach the region annotations for modality `M` (replacing any already
    /// set for `M`). Call once per modality present in the document.
    #[must_use]
    pub fn with_annotations<M: Modality>(mut self, annotations: Annotations<M>) -> Self {
        self.annotations = self.annotations.with(annotations);
        self
    }

    /// Override the orchestrator's run-wide [`Scope`] for this analysis only.
    ///
    /// The override replaces the run-wide scope wholesale (it is not merged);
    /// unset, the orchestrator's own scope applies.
    ///
    /// [`Scope`]: elide_core::recognition::Scope
    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::recognition::annotation::Exclusion;

    use super::*;

    #[test]
    fn annotation_set_routes_by_modality_and_defaults_empty() {
        let text_annos =
            Annotations::<Text>::new().with_exclusion(Exclusion::new(TextLocation::new(0, 5)));
        let set = AnnotationSet::default().with(text_annos);

        // The Text regions come back...
        assert_eq!(set.get::<Text>().exclusions.len(), 1);
        // ...and re-fetching is stable (get clones, doesn't consume).
        assert_eq!(set.get::<Text>().exclusions.len(), 1);
    }

    #[test]
    fn annotation_set_unset_modality_is_empty() {
        let set = AnnotationSet::default();
        let annos = set.get::<Text>();
        assert!(annos.inclusions.is_empty() && annos.exclusions.is_empty());
    }

    #[test]
    fn later_with_replaces_same_modality() {
        let set = AnnotationSet::default()
            .with(Annotations::<Text>::new().with_exclusion(Exclusion::new(TextLocation::new(0, 5))))
            .with(Annotations::<Text>::new()); // replaces
        assert!(set.get::<Text>().exclusions.is_empty(), "second with wins");
    }

    #[test]
    fn directives_default_has_no_scope_override() {
        assert!(Directives::new().scope.is_none());
        assert!(Directives::new().with_scope(Scope::new()).scope.is_some());
    }
}
