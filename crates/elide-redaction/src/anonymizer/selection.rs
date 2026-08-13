//! [`Selection`]: the operator pick for one redaction, and its serializable
//! [`SelectionView`].
//!
//! Where [`Rule`] is the *policy* ("this matcher binds this operator"), a
//! `Selection` is the *decision* that policy produced for a concrete
//! redaction: which operator won, why it matched, under what authority, and
//! over which entities. [`Anonymizer::select`] computes one per redaction —
//! after overlaps are merged and conflicts resolved — so a caller can inspect
//! and edit the picks before [`anonymize_selections`] applies them.
//!
//! A `Selection` carries a *live* operator, so it does not serialize. To review
//! or round-trip a pick — display it, ship it over a wire, edit it — take its
//! [`view`](Selection::view): a [`SelectionView`] is the same decision as plain
//! data (operator id, matched rule, attribution, covered entities), with no
//! operator and no modality. A caller rebuilds a `Selection` from a view by
//! resolving the operator id back to a live operator (through a registry it
//! wired with any runtime capabilities) and calling [`Selection::new`].
//!
//! A `Selection` is deliberately *leaner* than the [`Redaction`] event it
//! eventually produces: it carries only what is needed to make and review a
//! pick, not the full post-hoc audit payload (leak profile, key id) the
//! event records once the operator has run.
//!
//! [`Rule`]: super::Rule
//! [`Anonymizer::select`]: super::Anonymizer::select
//! [`anonymize_selections`]: super::Anonymizer::anonymize_selections
//! [`Redaction`]: elide_core::entity::audit::AuditKind::Redaction

use std::sync::Arc;

use elide_core::entity::audit::{Attribution, RuleMatch};
use elide_core::modality::Modality;
use elide_core::operator::{Operator, OperatorId};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The operator pick for one redaction.
///
/// One `Selection` is one redaction the anonymizer will perform: a single
/// operator run over one (possibly merged) span. It carries the live
/// [`operator`](Self::operator) that won, the [`entities`](Self::entities) it
/// covers (more than one when overlapping detections were merged), the
/// [rule](Self::matched_by) that chose it, and any policy
/// [`attribution`](Self::attribution). Overlaps are already merged and
/// conflicts resolved by the time a `Selection` exists, so apply just runs each
/// one — it never re-clusters. A caller may swap a selection's operator between
/// [`select`] and apply to override the pick; apply runs whatever the selection
/// carries.
///
/// # Reviewing and round-tripping
///
/// A `Selection` holds a trait-object operator, so it does not serialize. Call
/// [`view`](Self::view) for a [`SelectionView`] — the same decision as plain,
/// serializable data — to display, ship, or edit a pick. To turn a view back
/// into a `Selection`, resolve its [`operator_id`](SelectionView::operator_id)
/// to a live operator (typically through a registry) and call [`new`](Self::new).
///
/// [`select`]: super::Anonymizer::select
/// [`new`]: Self::new
pub struct Selection<M: Modality> {
    /// The live operator that will run.
    operator: Arc<dyn Operator<M>>,
    /// The entities this redaction covers. More than one when overlapping
    /// detections were merged into a single span.
    entities: Vec<Uuid>,
    /// Which selection rule chose this operator — the automatic "why".
    matched_by: RuleMatch,
    /// The matched rule's policy rationale, when it carried one.
    attribution: Option<Attribution>,
}

impl<M: Modality> Selection<M> {
    /// Build a selection: the operator the rules resolved (or a caller
    /// rebuilt from a [`SelectionView`]), the entities it covers, and the
    /// provenance of *why* it matched.
    pub fn new(
        operator: Arc<dyn Operator<M>>,
        entities: Vec<Uuid>,
        matched_by: RuleMatch,
        attribution: Option<Attribution>,
    ) -> Self {
        Self {
            operator,
            entities,
            matched_by,
            attribution,
        }
    }

    /// The live operator that will run.
    pub fn operator(&self) -> &Arc<dyn Operator<M>> {
        &self.operator
    }

    /// Identity (name + version) of the operator that won.
    pub fn operator_id(&self) -> OperatorId {
        self.operator.id()
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

    /// The serializable, modality-free view of this pick — the same decision
    /// as plain data, for display, wire transport, or review.
    ///
    /// The view drops the live operator, keeping its [`OperatorId`] as the
    /// stand-in a caller resolves back to an operator when rebuilding a
    /// `Selection`. See [`SelectionView`].
    pub fn view(&self) -> SelectionView {
        SelectionView {
            operator_id: self.operator.id(),
            entities: self.entities.clone(),
            matched_by: self.matched_by.clone(),
            attribution: self.attribution.clone(),
        }
    }
}

/// The serializable view of a [`Selection`] — one redaction pick as plain data.
///
/// The same decision a [`Selection`] carries, minus the live operator and the
/// modality: the [`operator_id`](Self::operator_id) that won, the
/// [`entities`](Self::entities) it covers, the [rule](Self::matched_by) that
/// chose it, and any policy [`attribution`](Self::attribution). This is what a
/// review tool displays, a wire protocol ships, and a reviewer edits.
///
/// A view is `Serialize + Deserialize`: it round-trips cleanly because every
/// field is plain data. To apply a (possibly edited) view, resolve its
/// `operator_id` to a live operator — through a registry wired with any runtime
/// capabilities (keys, a vault) — and build a [`Selection`] with
/// [`Selection::new`]. The `operator_id` is only the operator's *identity*; the
/// operator's configuration travels alongside it in the caller's policy, not in
/// the view.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SelectionView {
    /// Identity (name + version) of the operator that won.
    pub operator_id: OperatorId,
    /// The entities this redaction covers, by id. More than one when
    /// overlapping detections were merged into a single span.
    pub entities: Vec<Uuid>,
    /// Which selection rule chose this operator.
    pub matched_by: RuleMatch,
    /// The matched rule's policy rationale, when it carried one.
    pub attribution: Option<Attribution>,
}
