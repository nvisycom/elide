//! The entities a stream's flagged hidden content makes: the items its format
//! labels (metadata fields, comments, their authors, tracked deletions), made
//! entities as the document's [`HiddenContentPolicy`] says, before and beside
//! any recognizer, and the rule that erases them when no caller rule says how.

use std::any::TypeId;

use elide_codec::{FlaggedItem, TypedStream};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::entity::audit::{Attribution, AuditEvent, FlaggedEvent};
use elide_core::modality::metadata::Metadata;
use elide_core::modality::{Modality, ModalityLocation, Overlap};
use elide_core::primitive::Confidence;
use elide_core::recognition::Scope;
use elide_core::recognition::annotation::Annotations;
use elide_core::redaction::{Disposition, HiddenContentPolicy, Operator};
use elide_redaction::Rule;
use elide_redaction::operators::Erase;

/// The attribution the [`erase_rule`] records on what it erases.
const FLAGGED_ATTRIBUTION: &str = "flagged-hidden-content";

/// The rule every pipeline's anonymizer ends in: erase each flagged entity no
/// earlier rule covers.
///
/// A flagged item is hidden content the policy chose to redact, so it is not
/// kept merely because the caller's rules name none of its labels. A caller
/// rule matching it still wins, [`Keep`](elide_redaction::operators::Keep)
/// included.
pub(crate) fn erase_rule<M: Modality>() -> Rule<M>
where
    Erase: Operator<M>,
{
    Rule::predicate(|cx| cx.entity.audit.is_flagged(), Erase)
        .because(Attribution::freeform(FLAGGED_ATTRIBUTION))
}

/// One entity per item `stream` flags that `policy` selects and no exclusion
/// covers whole.
///
/// Under [`FlagEverything`](Disposition::FlagEverything) every item is
/// selected. Under [`ScanAsContent`](Disposition::ScanAsContent) an item of a
/// metadata stream is selected when the scope's catalog declares its label,
/// since its label is what the field is, whatever a recognizer finds in its
/// value; an item of a text stream is not, since recognizers scan it like body
/// text.
///
/// An exclusion inside an item does not spare it: the item is one entity, and
/// the rest of it has no other detection to fall back on.
pub(crate) fn entities<M: Modality>(
    stream: &TypedStream<M>,
    scope: &Scope,
    policy: HiddenContentPolicy,
    annotations: &Annotations<M>,
) -> Result<Vec<Entity<M>>> {
    let source = stream.format();
    Ok(stream
        .flagged()?
        .into_iter()
        .filter(|item| selects(policy, scope, item))
        .filter(|item| {
            !annotations
                .exclusions
                .iter()
                .any(|exclusion| exclusion.location.overlap(&item.location) == Overlap::Contains)
        })
        .filter_map(|item| entity(source.as_str(), item))
        .collect())
}

/// Whether `policy` makes `item` an entity.
fn selects<M: Modality>(policy: HiddenContentPolicy, scope: &Scope, item: &FlaggedItem<M>) -> bool {
    match policy.disposition(item.kind) {
        Some(Disposition::FlagEverything) => true,
        Some(Disposition::ScanAsContent) => {
            TypeId::of::<M>() == TypeId::of::<Metadata>() && scope.catalog.contains(&item.label)
        }
        Some(Disposition::KeepOnlyEssential) | None => false,
    }
}

/// The entity for `item`, attributed to the format `source` that flagged it.
///
/// A flagged item is present or it is not, nothing probabilistic to weigh, so
/// the entity carries [`Confidence::MAX`].
fn entity<M: Modality>(source: &str, item: FlaggedItem<M>) -> Option<Entity<M>> {
    let event = AuditEvent::flagged(
        source.to_owned(),
        Confidence::MAX,
        item.location.clone(),
        FlaggedEvent { content: item.kind },
    );
    Entity::builder()
        .with_label(item.label)
        .with_location(item.location)
        .with_confidence(Confidence::MAX)
        .with_event(event)
        .build()
}
