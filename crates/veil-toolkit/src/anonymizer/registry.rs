//! The [`OperatorRegistry`] — a label→operator map with a fallback,
//! resolving which operator hides which entity.

use std::collections::HashMap;
use std::sync::Arc;

use veil_core::entity::LabelRef;
use veil_core::modality::Modality;
use veil_core::redaction::Operator;

use super::dyn_operator::DynOperator;

/// Maps entity labels to the operators that hide them, with an optional
/// fallback for unmapped labels.
///
/// This is the minimal, Presidio-style "what to hide" layer: a
/// `label → operator` table plus a catch-all. Resolving an entity's
/// label yields the operator to run; an unmapped label falls back, or is
/// left untouched if there is no fallback. No conditions, no rules — a
/// rule engine, if wanted, lives a layer up.
pub(crate) struct OperatorRegistry<M: Modality> {
    by_label: HashMap<LabelRef, Arc<dyn DynOperator<M>>>,
    fallback: Option<Arc<dyn DynOperator<M>>>,
}

impl<M: Modality> OperatorRegistry<M> {
    /// An empty registry.
    pub(crate) fn new() -> Self {
        Self {
            by_label: HashMap::new(),
            fallback: None,
        }
    }

    /// Register `operator` for `label`.
    pub(crate) fn insert<O: Operator<M> + 'static>(&mut self, label: LabelRef, operator: O) {
        self.by_label.insert(label, Arc::new(operator));
    }

    /// Set the fallback operator for unmapped labels.
    pub(crate) fn set_fallback<O: Operator<M> + 'static>(&mut self, operator: O) {
        self.fallback = Some(Arc::new(operator));
    }

    /// Resolve the operator for `label`: its mapping, else the fallback,
    /// else `None`.
    pub(crate) fn resolve(&self, label: &LabelRef) -> Option<&Arc<dyn DynOperator<M>>> {
        self.by_label.get(label).or(self.fallback.as_ref())
    }
}

impl<M: Modality> Default for OperatorRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}
