//! Candidate → [`Entity`] lifting shared by [`DefaultPrompt`] and
//! [`FilePrompt`].
//!
//! Both prompts produce the same `{TextCandidates,VlmCandidates}`
//! JSON shape and walk it the same way to emit entities. The only
//! axis they differ on is whether they apply a [`LabelMap`] +
//! `labels_to_ignore` filter on the model's emitted label string —
//! [`DefaultPrompt`] passes an empty map + empty slice (no
//! filtering); [`FilePrompt`] passes its loaded config.
//!
//! [`DefaultPrompt`]: super::default_prompt::DefaultPrompt
//! [`FilePrompt`]: super::file_prompt::FilePrompt

use nvisy_core::entity::{Entity, EntityLabelRef, ModelProvenance, TrailProvenance, TrailStep};
use nvisy_core::modality::{Image, ImageLocation, Text, TextLocation};
use nvisy_core::primitive::Confidence;
use nvisy_core::recognition::{LabelMap, RecognizerInput};

use super::candidates::{TextCandidate, VlmCandidate};
use super::localize::{UnresolvedCandidatePolicy, localize_all};

/// Default confidence assigned to a candidate when the LLM didn't
/// score it.
const DEFAULT_CONFIDENCE: f64 = 0.5;

/// Lift a parsed text-candidate batch into `Entity<Text>` values.
///
/// `label_map` and `labels_to_ignore` together implement the
/// model-label → canonical-name translation. Pass an empty map + an
/// empty slice for no filtering.
pub(super) fn lift_text(
    input: &RecognizerInput<Text>,
    candidates: Vec<TextCandidate>,
    label_map: &LabelMap,
    labels_to_ignore: &[String],
) -> Vec<Entity<Text>> {
    let text = input.data.text.as_str();
    let localized = localize_all(text, candidates, UnresolvedCandidatePolicy::default());
    let model = ModelProvenance::new("llm".to_owned());

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
        let Some(confidence) = Confidence::new(raw.clamp(0.0, 1.0)) else {
            continue;
        };
        let location = TextLocation::new(l.start_offset, l.end_offset);
        let reason = format!("llm identified {label}");
        let step = TrailStep::recognition(
            "llm-ner",
            confidence,
            TrailProvenance::Model(model.clone()),
            reason,
        );

        let mut b = Entity::builder()
            .with_label(label)
            .with_trail(vec![step])
            .with_confidence(confidence)
            .with_location(location);
        if let Some(id) = l.candidate.entity_id {
            b = b.with_entity_id(id);
        }
        out.push(b.build().expect("required fields provided"));
    }
    out
}

/// Lift a parsed VLM-candidate batch into `Entity<Image>` values.
pub(super) fn lift_image(
    input: &RecognizerInput<Image>,
    candidates: Vec<VlmCandidate>,
    label_map: &LabelMap,
    labels_to_ignore: &[String],
) -> Vec<Entity<Image>> {
    let dims = input.data.dims;
    let model = ModelProvenance::new("llm".to_owned());

    let mut out = Vec::with_capacity(candidates.len());
    for d in candidates {
        if labels_to_ignore.iter().any(|l| l == &d.label) {
            continue;
        }
        let label = label_map
            .lookup(&d.label)
            .cloned()
            .unwrap_or_else(|| EntityLabelRef::from(d.label.clone()));
        let raw = d.confidence.unwrap_or(DEFAULT_CONFIDENCE);
        let Some(confidence) = Confidence::new(raw.clamp(0.0, 1.0)) else {
            continue;
        };
        let bbox = d.bbox.to_pixel(dims);
        let location = ImageLocation::new(bbox);
        let reason = format!("vlm identified {label}");
        let step = TrailStep::recognition(
            "llm-vlm",
            confidence,
            TrailProvenance::Model(model.clone()),
            reason,
        );
        let entity = Entity::builder()
            .with_label(label)
            .with_trail(vec![step])
            .with_confidence(confidence)
            .with_location(location)
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
) -> Option<EntityLabelRef> {
    if let Some(model_label) = typed {
        if labels_to_ignore.iter().any(|l| l == model_label) {
            return None;
        }
        return Some(
            label_map
                .lookup(model_label)
                .cloned()
                .unwrap_or_else(|| EntityLabelRef::from(model_label.to_owned())),
        );
    }
    if labels_to_ignore.iter().any(|l| l == value) {
        return None;
    }
    label_map.lookup(value).cloned()
}
