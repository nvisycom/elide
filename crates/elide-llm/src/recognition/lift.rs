//! Candidate to [`Entity`] lifting shared by [`DefaultPrompt`] and
//! [`FilePrompt`].
//!
//! Both prompts produce the same `{TextCandidates,VlmCandidates}`
//! JSON shape and walk it the same way to emit entities. The only
//! axis they differ on is whether they apply a [`LabelMap`] +
//! `labels_to_ignore` filter on the model's emitted label string:
//! [`DefaultPrompt`] passes an empty map + empty slice (no
//! filtering); [`FilePrompt`] passes its loaded config.
//!
//! [`DefaultPrompt`]: super::default_prompt::DefaultPrompt
//! [`FilePrompt`]: super::file_prompt::FilePrompt

use elide_core::entity::{Entity, EntityCoRef, LabelRef};
use elide_core::modality::image::{Image, ImageData, ImageLocation};
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::primitive::{Confidence, UnitBoundingBox};
use elide_core::provenance::{Event, ModelEvent};
use elide_core::recognition::LabelMap;

use super::candidates::{TextCandidate, VlmCandidate};
use super::localize::{UnresolvedCandidatePolicy, localize_all};

/// Default confidence assigned to a candidate when the LLM didn't
/// score it.
const DEFAULT_CONFIDENCE: f64 = 0.5;

/// Lift a parsed text-candidate batch into `Entity<Text>` values.
///
/// `label_map` and `labels_to_ignore` together implement the
/// model-label to canonical-name translation. Pass an empty map + an
/// empty slice for no filtering.
pub(super) fn lift_text(
    data: &TextData,
    candidates: Vec<TextCandidate>,
    label_map: &LabelMap,
    labels_to_ignore: &[String],
) -> Vec<Entity<Text>> {
    let text = data.text.as_str();
    let localized = localize_all(text, candidates, UnresolvedCandidatePolicy::default());

    let mut out = Vec::with_capacity(localized.len());
    for l in localized {
        let Some(label) = resolve_text_label(
            l.candidate.entity_type.as_deref(),
            l.candidate.value.as_str(),
            label_map,
            labels_to_ignore,
        ) else {
            continue;
        };
        let raw = l.candidate.confidence.unwrap_or(DEFAULT_CONFIDENCE);
        let Some(confidence) = Confidence::new(raw.clamp(0.0, 1.0) as f32) else {
            continue;
        };
        let location = TextLocation::new(l.start_offset, l.end_offset);
        let reason = format!("llm-ner identified {}", label.as_str());
        let event = Event::model(
            "llm-ner",
            confidence,
            location.clone(),
            ModelEvent {
                name: "llm-ner".into(),
                ..ModelEvent::default()
            },
        )
        .with_reason(reason);

        let mut builder = Entity::builder()
            .with_label(label)
            .with_location(location)
            .with_confidence(confidence)
            .with_event(event);
        // The model groups mentions of the same real-world entity under a
        // shared id; carry it onto the entity as a coreference cluster.
        if let Some(id) = l.candidate.entity_id.clone() {
            builder = builder.with_coref(EntityCoRef::new(id));
        }
        out.push(builder.build().expect("required fields provided"));
    }
    out
}

/// Lift a parsed VLM-candidate batch into `Entity<Image>` values.
pub(super) fn lift_image(
    data: &ImageData,
    candidates: Vec<VlmCandidate>,
    label_map: &LabelMap,
    labels_to_ignore: &[String],
) -> Vec<Entity<Image>> {
    let dims = data.dimensions;

    let mut out = Vec::with_capacity(candidates.len());
    for d in candidates {
        if labels_to_ignore.iter().any(|l| l == &d.label) {
            continue;
        }
        let label = label_map
            .get(&d.label)
            .cloned()
            .unwrap_or_else(|| LabelRef::new(d.label.clone()));
        let raw = d.confidence.unwrap_or(DEFAULT_CONFIDENCE);
        let Some(confidence) = Confidence::new(raw.clamp(0.0, 1.0) as f32) else {
            continue;
        };
        let bbox = UnitBoundingBox::from(d.bbox).denormalize(dims);
        let location = ImageLocation::new(bbox);
        let reason = format!("llm-vlm identified {}", label.as_str());
        let event = Event::model(
            "llm-vlm",
            confidence,
            location.clone(),
            ModelEvent {
                name: "llm-vlm".into(),
                ..ModelEvent::default()
            },
        )
        .with_reason(reason);
        let entity = Entity::builder()
            .with_label(label)
            .with_location(location)
            .with_confidence(confidence)
            .with_event(event)
            .build()
            .expect("required fields provided");
        out.push(entity);
    }
    out
}

/// Pick the canonical label name for a text candidate.
///
/// Priority order: (1) the model's typed label, after label-map +
/// ignore-list filtering; (2) literal-value lookup in the label
/// map (covers raw-string-label backends); (3) drop.
fn resolve_text_label(
    typed: Option<&str>,
    value: &str,
    label_map: &LabelMap,
    labels_to_ignore: &[String],
) -> Option<LabelRef> {
    if let Some(model_label) = typed {
        if labels_to_ignore.iter().any(|l| l == model_label) {
            return None;
        }
        return Some(
            label_map
                .get(model_label)
                .cloned()
                .unwrap_or_else(|| LabelRef::new(model_label.to_owned())),
        );
    }
    if labels_to_ignore.iter().any(|l| l == value) {
        return None;
    }
    label_map.get(value).cloned()
}
