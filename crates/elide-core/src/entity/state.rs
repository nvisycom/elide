//! [`State`]: what an [`Entity`](super::Entity) says, as its trail records
//! it.

use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::audit::{AuditEvent, AuditKind, ManualIntent};
use super::{EntityCoRef, LabelRef};
use crate::modality::Modality;
#[cfg(feature = "serde")]
use crate::modality::ModalityLocation;
use crate::primitive::{Confidence, LanguageTag};

/// What an entity says, read from its trail: set at its birth, and changed by
/// each later event on its line.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema")
)]
pub(crate) struct State<M: Modality> {
    /// What kind of sensitive information this is (resolved via a
    /// [`LabelCatalog`](super::LabelCatalog)).
    pub(crate) label: LabelRef,
    /// Location of the entity within the medium (fused, if it came from more
    /// than one detection).
    pub(crate) location: M::Location,
    /// Effective confidence in `0.0..=1.0`, the confidence of the trail's
    /// latest event.
    pub(crate) confidence: Confidence,
    /// Coreference identifier, if a recognizer resolved this entity as one
    /// mention of a cluster. Entities sharing an [`EntityCoRef`] denote the
    /// same real-world thing.
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) coref: Option<EntityCoRef>,
    /// The language of this entity's surrounding text, when a recognizer
    /// resolved one. `None` when unknown or language-agnostic.
    #[cfg_attr(feature = "serde", serde(default))]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub(crate) language: Option<LanguageTag>,
    /// Byte range of the match in the *recognized text* it was found in (the
    /// OCR layout text, the audio transcript, or the text payload itself),
    /// the stable key back into that enrichment artifact, where the rich
    /// context lives (which OCR block, which speaker) that the geometric
    /// location cannot hold. `None` for entities not found via text
    /// recognition (e.g. a VLM box). An audit key, not a coordinate:
    /// redaction uses the location; an audit uses this with the artifact.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub(crate) recognized_range: Option<Range<usize>>,
}

impl<M: Modality> State<M> {
    /// An entity's state at its birth: `label` at `location`, with
    /// `confidence`, and nothing else known yet.
    pub(crate) fn new(label: LabelRef, location: M::Location, confidence: Confidence) -> Self {
        Self {
            label,
            location,
            confidence,
            coref: None,
            language: None,
            recognized_range: None,
        }
    }

    /// The state `event` starts an entity in, when it is a recognition or a
    /// reviewer's inclusion.
    pub(crate) fn born(event: &AuditEvent<M>) -> Option<Self> {
        let (label, location, range) = match &event.kind {
            AuditKind::Pattern(p) => (&p.label, &p.location, &p.recognized_range),
            AuditKind::Model(m) => (&m.label, &m.location, &m.recognized_range),
            AuditKind::Flagged(f) => (&f.label, &f.location, &None),
            AuditKind::Manual(manual) => match &manual.intent {
                ManualIntent::Include { label } => (label, &manual.location, &None),
                _ => return None,
            },
            _ => return None,
        };
        let mut state = Self::new(label.clone(), location.clone(), event.confidence);
        state.recognized_range.clone_from(range);
        Some(state)
    }

    /// Take the effect of `event`, which follows the entity's birth on its
    /// line: a reviewer's decision sets the label or location it names, a
    /// fusion the fused location, an annotation each field it sets, and every
    /// event the confidence.
    pub(crate) fn apply(&mut self, event: &AuditEvent<M>) {
        match &event.kind {
            AuditKind::Manual(manual) => match &manual.intent {
                ManualIntent::Include { label } => {
                    self.label = label.clone();
                    self.location = manual.location.clone();
                }
                ManualIntent::Relabel { label } => self.label = label.clone(),
                ManualIntent::Relocate => self.location = manual.location.clone(),
                ManualIntent::Suppress => {}
            },
            AuditKind::Fusion(fusion) => self.location = fusion.location.clone(),
            AuditKind::Annotation(annotation) => {
                if let Some(location) = &annotation.location {
                    self.location = location.clone();
                }
                if let Some(range) = &annotation.recognized_range {
                    self.recognized_range = Some(range.clone());
                }
                if let Some(language) = &annotation.language {
                    self.language = Some(language.clone());
                }
                if let Some(coref) = &annotation.coref {
                    self.coref = Some(coref.clone());
                }
            }
            _ => {}
        }
        self.confidence = event.confidence;
    }

    /// Whether `other` says the same as this state.
    #[cfg(feature = "serde")]
    pub(crate) fn agrees(&self, other: &Self) -> bool {
        self.label == other.label
            && self.location.hash() == other.location.hash()
            && self.confidence == other.confidence
            && self.coref == other.coref
            && self.language == other.language
            && self.recognized_range == other.recognized_range
    }
}
