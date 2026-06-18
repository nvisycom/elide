//! Structured-output candidate types: the typed schemas the model
//! is asked to produce.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use veil_core::primitive::UnitBoundingBox;

/// Serde wrapper matching the model's `{"entities": [...]}`
/// response for the [`Text`] modality.
///
/// [`Text`]: veil_core::modality::text::Text
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub(super) struct TextCandidates {
    /// Detected candidates.
    pub entities: Vec<TextCandidate>,
}

/// One entity candidate produced by the model for the text
/// modality. Carries the surface form (`value`) plus a surrounding
/// `context` snippet the recognizer uses to localize the value back
/// into a byte range in the source text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub(super) struct TextCandidate {
    /// Model-assigned identifier for the underlying real-world
    /// entity. Stable across coreferent mentions within one call.
    #[serde(default)]
    pub entity_id: Option<String>,
    /// Label name. Missing (`None`) means the model declined to
    /// type the candidate; the recognizer drops these.
    pub entity_type: Option<String>,
    /// The matched text value: the literal surface form to flag.
    pub value: String,
    /// Model-asserted confidence in `[0.0, 1.0]`.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Short surrounding text intended to uniquely locate `value`.
    /// Missing or non-unique `context` causes the candidate to be
    /// dropped per the recognizer's policy.
    #[serde(default)]
    pub context: Option<String>,
    /// Brief description of the real-world entity (advisory).
    #[serde(default)]
    pub description: Option<String>,
}

/// Serde wrapper matching the model's `{"entities": [...]}`
/// response for the [`Image`] modality.
///
/// [`Image`]: veil_core::modality::image::Image
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub(super) struct VlmCandidates {
    pub entities: Vec<VlmCandidate>,
}

/// One image entity discovered by the VLM. Bounding box is
/// normalised (`[0, 1]`); the recognizer scales to pixel
/// coordinates using the source image's dimensions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub(super) struct VlmCandidate {
    pub label: String,
    #[serde(flatten)]
    pub bbox: UnitBox,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Wire shape of a VLM bounding box, in normalised `[0, 1]`
/// coordinates. Mirrors [`UnitBoundingBox`] but carries the
/// `JsonSchema` derive the structured-output schema needs (the core's
/// [`UnitBoundingBox`] does not depend on `schemars`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub(super) struct UnitBox {
    /// Top-left x in `0.0..=1.0` (fraction of image width).
    pub x: f64,
    /// Top-left y in `0.0..=1.0` (fraction of image height).
    pub y: f64,
    /// Width in `0.0..=1.0` (fraction of image width).
    pub width: f64,
    /// Height in `0.0..=1.0` (fraction of image height).
    pub height: f64,
}

impl From<UnitBox> for UnitBoundingBox {
    fn from(b: UnitBox) -> Self {
        UnitBoundingBox::new(b.x, b.y, b.width, b.height)
    }
}
