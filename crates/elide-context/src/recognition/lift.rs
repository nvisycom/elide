//! Lifting stream-positioned [`EntityDraft`]s to located [`Entity`]s.
//!
//! The one generic step that turns a draft's ephemeral stream byte range
//! into a native location via the modality's [`locate`], shared by the
//! bare and enhanced recognition paths.
//!
//! [`locate`]: elide_core::modality::TextRecognizable::locate

use elide_core::entity::Entity;
use elide_core::entity::provenance::Event;
use elide_core::modality::TextRecognizable;
use elide_core::recognition::Artifacts;

use super::EntityDraft;

/// Lift a stream-positioned [`EntityDraft`] to a located [`Entity`].
///
/// Drops it (returning `None`) when its stream range can't be placed in the
/// medium — the same fallible-locate behaviour bare recognizers use.
///
/// The draft's `stream_range` is consumed here; the resulting entity carries
/// only the native location.
pub fn lift<M: TextRecognizable>(
    draft: EntityDraft,
    data: &M::Data,
    artifacts: &Artifacts,
) -> Option<Entity<M>> {
    let Some(location) = M::locate(draft.stream_range, data, artifacts) else {
        // The match's stream range maps to no native location (an OCR /
        // transcript range no enrichment covers); drop it rather than emit
        // a placeless entity.
        tracing::warn!(
            label = draft.label.as_str(),
            "could not place a match in the source; dropping it",
        );
        return None;
    };
    let event = Event::pattern(
        draft.event.source,
        draft.confidence,
        location.clone(),
        draft.event.pattern,
    )
    .with_reason(draft.event.reason);
    let mut builder = Entity::builder()
        .with_label(draft.label)
        .with_location(location)
        .with_confidence(draft.confidence)
        .with_event(event);
    if let Some(coref) = draft.coref {
        builder = builder.with_coref(coref);
    }
    Some(builder.build().expect("required fields provided"))
}

/// [`lift`] every draft to a located [`Entity`], dropping the unplaceable.
///
/// The no-enhancement lift path: a [`StreamRecognizer`] that wants to be a
/// plain [`Recognizer`] runs `find` then this. [`Enhanced`] does not call it
/// — it lifts with extra bookkeeping so it can attach boost provenance.
///
/// [`StreamRecognizer`]: super::StreamRecognizer
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`Enhanced`]: super::Enhanced
pub fn lift_all<M: TextRecognizable>(
    drafts: Vec<EntityDraft>,
    data: &M::Data,
    artifacts: &Artifacts,
) -> Vec<Entity<M>> {
    drafts
        .into_iter()
        .filter_map(|draft| lift::<M>(draft, data, artifacts))
        .collect()
}
