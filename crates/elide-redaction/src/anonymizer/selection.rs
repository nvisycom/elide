//! [`Selection`]: the operator pick for one redaction.
//!
//! Where [`Rule`] is the *policy* ("this matcher binds this operator"), a
//! `Selection` is the *decision* that policy produced for a concrete
//! redaction: which operator won, why it matched, under what authority, and
//! over which entities. [`Anonymizer::select`] computes one per redaction —
//! after overlaps are merged and conflicts resolved — so a caller can inspect
//! and edit the picks before [`anonymize_selections`] applies them.
//!
//! A `Selection` is deliberately *leaner* than the [`Redaction`] event it
//! eventually produces: it carries only what is needed to make and review a
//! pick, not the full post-hoc audit payload (leak profile, key id) the
//! event records once the operator has run.
//!
//! [`Rule`]: super::Rule
//! [`Anonymizer::select`]: super::Anonymizer::select
//! [`anonymize_selections`]: super::Anonymizer::anonymize_selections
//! [`Redaction`]: elide_core::entity::provenance::EventKind::Redaction

use std::sync::Arc;

use elide_core::entity::provenance::{Attribution, RuleMatch};
use elide_core::modality::Modality;
use elide_core::operator::{Operator, OperatorId};
#[cfg(feature = "serde")]
use serde::Serialize;
use uuid::Uuid;

/// The operator pick for one redaction.
///
/// One `Selection` is one redaction the anonymizer will perform: a single
/// operator run over one (possibly merged) span. It names the
/// [`operator`](Self::operator_id) that won, the [`entities`](Self::entities)
/// it covers (more than one when overlapping detections were merged), the
/// [rule](Self::matched_by) that chose it, and any policy
/// [`attribution`](Self::attribution). Overlaps are already merged and
/// conflicts resolved by the time a `Selection` exists, so apply just runs
/// each one — it never re-clusters. A caller may swap a selection's operator
/// between [`select`] and apply to override the pick; apply runs whatever the
/// selection carries.
///
/// # Serialization
///
/// A `Selection` is `Serialize` — it emits its *policy* view (operator id,
/// which rule matched, attribution, covered entities) so a review tool can
/// display the pick. It is **not** `Deserialize`: the live operator it carries
/// cannot be reconstructed by serde alone (an operator's config and any
/// runtime capabilities it needs are rebuilt through a registry, not
/// deserialized in place). A round-tripped selection is therefore rebuilt from
/// its serialized policy, not deserialized back into this type directly.
///
/// [`select`]: super::Anonymizer::select
// NOTE: no `JsonSchema` derive yet. `Selection` serializes only its policy
// view today; a wire-complete, schema-emitting form (operator id + config)
// lands with operator config-serialization. See issue #165.
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(bound(serialize = "")))]
pub struct Selection<M: Modality> {
    /// The live operator that will run. Skipped in serialization: it is a
    /// trait object with no serde form and no default, carried only for the
    /// in-process apply path.
    #[cfg_attr(feature = "serde", serde(skip))]
    operator: Arc<dyn Operator<M>>,
    /// Identity (name + version) of the operator that won — the serializable
    /// stand-in for the live operator above.
    operator_id: OperatorId,
    /// The entities this redaction covers. More than one when overlapping
    /// detections were merged into a single span.
    entities: Vec<Uuid>,
    /// Which selection rule chose this operator — the automatic "why".
    matched_by: RuleMatch,
    /// The matched rule's policy rationale, when it carried one.
    attribution: Option<Attribution>,
}

impl<M: Modality> Selection<M> {
    /// Build a selection: the operator the rules resolved, the entities it
    /// covers, and the provenance of *why* it matched.
    pub(crate) fn new(
        operator: Arc<dyn Operator<M>>,
        entities: Vec<Uuid>,
        matched_by: RuleMatch,
        attribution: Option<Attribution>,
    ) -> Self {
        let operator_id = operator.id();
        Self {
            operator,
            operator_id,
            entities,
            matched_by,
            attribution,
        }
    }

    /// The live operator that will run.
    pub(crate) fn operator(&self) -> &Arc<dyn Operator<M>> {
        &self.operator
    }

    /// Identity of the operator that won.
    pub fn operator_id(&self) -> &OperatorId {
        &self.operator_id
    }

    /// The entities this redaction covers.
    pub fn entities(&self) -> &[Uuid] {
        &self.entities
    }

    /// Which selection rule chose this operator.
    pub fn matched_by(&self) -> &RuleMatch {
        &self.matched_by
    }

    /// The matched rule's policy attribution, if any.
    pub fn attribution(&self) -> Option<&Attribution> {
        self.attribution.as_ref()
    }
}
