//! The entities a stream's flagged hidden content makes: the items its format
//! labels (metadata fields, comments, their authors, tracked deletions), made
//! entities as the document's [`HiddenContentPolicy`] says, before and beside
//! any recognizer, and the rule that removes them when no caller rule says how.

use std::any::{Any, TypeId};

use elide_codec::{FlaggedItem, TypedStream};
use elide_core::entity::Entity;
use elide_core::entity::audit::{Attribution, AuditEvent};
use elide_core::modality::Modality;
use elide_core::modality::metadata::Metadata;
use elide_core::modality::text::Text;
use elide_core::primitive::Confidence;
use elide_core::recognition::Scope;
use elide_core::redaction::{Disposition, HiddenContentPolicy, Operator};
use elide_core::{Error, Result};
use elide_redaction::operators::Remove;
use elide_redaction::{MatchContext, Rule};

/// The attribution the [`erase_rule`] records on what it erases.
const FLAGGED_ATTRIBUTION: &str = "flagged-hidden-content";

/// The rule a pipeline's anonymizer ends in: remove each flagged entity no
/// earlier rule covers.
///
/// A flagged item is hidden content the policy chose to redact, so it is not
/// kept merely because the caller's rules name none of its labels. A caller
/// rule matching it still wins, [`Keep`](elide_redaction::operators::Keep)
/// included. Only text and metadata streams flag items, so only their
/// modalities have the rule; [`entities`] refuses an item flagged in any other.
pub(crate) fn erase_rule<M: Modality>() -> Option<Rule<M>> {
    let rule: Box<dyn Any> = if TypeId::of::<M>() == TypeId::of::<Text>() {
        Box::new(removing::<Text>())
    } else if TypeId::of::<M>() == TypeId::of::<Metadata>() {
        Box::new(removing::<Metadata>())
    } else {
        return None;
    };
    rule.downcast::<Rule<M>>().ok().map(|rule| *rule)
}

/// The rule removing each flagged entity of modality `M`.
fn removing<M: Modality>() -> Rule<M>
where
    Remove: Operator<M>,
{
    Rule::predicate(
        |cx: &MatchContext<'_, M>| cx.entity.audit.is_flagged(),
        Remove,
    )
    .because(Attribution::freeform(FLAGGED_ATTRIBUTION))
}

/// One entity per item `stream` flags that `policy` selects.
///
/// Under [`FlagEverything`](Disposition::FlagEverything) every item is
/// selected. Under [`ScanAsContent`](Disposition::ScanAsContent) an item of a
/// metadata stream is selected when the scope's catalog declares its label,
/// since its label is what the field is, whatever a recognizer finds in its
/// value; an item of a text stream is not, since recognizers scan it like body
/// text.
///
/// # Errors
///
/// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
/// if a selected item's modality has no [`erase_rule`], so it would be kept
/// unredacted.
pub(crate) fn entities<M: Modality>(
    stream: &TypedStream<'_, M>,
    scope: &Scope,
    policy: HiddenContentPolicy,
) -> Result<Vec<Entity<M>>> {
    let source = stream.format();
    let entities: Vec<_> = stream
        .flagged()?
        .into_iter()
        .filter(|item| selects(policy, scope, item))
        .map(|item| entity(source.as_str(), item))
        .collect();
    if !entities.is_empty() && erase_rule::<M>().is_none() {
        return Err(Error::capability_unavailable(format!(
            "{source} flags hidden content in {}, which has no rule to remove it",
            M::NAME
        )));
    }
    Ok(entities)
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
fn entity<M: Modality>(source: &str, item: FlaggedItem<M>) -> Entity<M> {
    let event = AuditEvent::flagged(
        source.to_owned(),
        Confidence::MAX,
        item.location.clone(),
        item.kind,
    );
    Entity::new(item.label, item.location).with_event(event)
}

#[cfg(test)]
mod tests {
    use elide_core::modality::Modality;
    use elide_core::modality::metadata::Metadata;
    use elide_core::modality::text::Text;
    #[cfg(feature = "image")]
    use elide_image::modality::Image;

    use super::erase_rule;

    fn has_rule<M: Modality>() -> bool {
        erase_rule::<M>().is_some()
    }

    /// Only the modalities whose streams flag items have the rule.
    #[test]
    fn text_and_metadata_have_the_erase_rule() {
        assert!(has_rule::<Text>());
        assert!(has_rule::<Metadata>());
        #[cfg(feature = "image")]
        assert!(!has_rule::<Image>());
    }
}
