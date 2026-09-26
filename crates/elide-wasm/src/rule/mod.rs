//! The redaction rules: which entities an [`Operator`]
//! acts on.
//!
//! Built with [`Rule::label`] (act on entities of one label) or [`Rule::fallback`]
//! (act on anything not matched by an earlier rule) and folded into an
//! [`Anonymizer`](crate::anonymizer::Anonymizer) with
//! [`Anonymizer::rule`](crate::anonymizer::Anonymizer::rule). The operator a rule
//! carries decides which modality it applies to.

use wasm_bindgen::prelude::*;

use crate::operator::{Operator, OperatorKind};

/// The rules an [`Rule`] handle may carry.
pub(crate) enum RuleKind {
    /// Act on entities of the given label id.
    Label { id: String, operator: OperatorKind },
    /// Act on any entity not matched by an earlier rule.
    Fallback { operator: OperatorKind },
}

/// A redaction rule, ready to fold into an anonymizer.
///
/// Built with [`Rule::label`] or [`Rule::fallback`] and consumed by
/// [`Anonymizer::rule`](crate::anonymizer::Anonymizer::rule).
#[wasm_bindgen]
pub struct Rule(pub(crate) RuleKind);

#[wasm_bindgen]
impl Rule {
    /// A rule that applies `operator` to every entity of `label` (a label id such
    /// as `"email_address"`). Consumes the operator.
    #[wasm_bindgen(js_name = label)]
    pub fn label(label: String, operator: Operator) -> Rule {
        Self(RuleKind::Label {
            id: label,
            operator: operator.into_kind(),
        })
    }

    /// A catch-all rule that applies `operator` to every entity not matched by an
    /// earlier rule. Consumes the operator.
    #[wasm_bindgen(js_name = fallback)]
    pub fn fallback(operator: Operator) -> Rule {
        Self(RuleKind::Fallback {
            operator: operator.into_kind(),
        })
    }
}
