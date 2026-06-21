//! Structured-output candidate types: the typed schemas the model is
//! asked to produce.
//!
//! These are the `T` in rig's `Extractor::<T>` — the backend asks the
//! model to fill them in, and the [`JsonSchema`] derive is what constrains
//! the output. One shape per modality; the recognizer localizes each
//! candidate's value into the source and builds the final entity.

use elide_core::modality::Modality;
use elide_core::modality::image::Image;
use elide_core::modality::text::Text;
use elide_core::primitive::UnitBoundingBox;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The structured-output candidate batch a backend extracts for modality
/// `M`: [`TextCandidates`] for [`Text`], [`ImageCandidates`] for [`Image`].
///
/// Bound on the modality so a generic backend can name the right shape
/// from `M` alone.
pub trait Candidates: Modality {
    /// The candidate-batch type the model produces for this modality.
    type Batch: JsonSchema
        + for<'a> Deserialize<'a>
        + Serialize
        + Default
        + Send
        + Sync
        + 'static;
}

impl Candidates for Text {
    type Batch = TextCandidates;
}

impl Candidates for Image {
    type Batch = ImageCandidates;
}

/// Serde wrapper matching the model's `{"entities": [...]}`
/// response for the [`Text`] modality.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextCandidates {
    /// Detected candidates.
    pub entities: Vec<TextCandidate>,
}

/// One entity candidate produced by the model for the text
/// modality. Carries the surface form (`value`) plus a surrounding
/// `context` snippet the recognizer uses to localize the value back
/// into a byte range in the source text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextCandidate {
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImageCandidates {
    /// Detected candidates.
    pub entities: Vec<ImageCandidate>,
}

/// One image entity discovered by the model. Bounding box is
/// normalised (`[0, 1]`); the recognizer scales to pixel
/// coordinates using the source image's dimensions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImageCandidate {
    /// Label name for the detected region.
    pub label: String,
    /// Normalised bounding box of the region.
    #[serde(flatten)]
    pub bbox: UnitBox,
    /// Model-asserted confidence in `[0.0, 1.0]`.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Brief description of the region (advisory).
    #[serde(default)]
    pub description: Option<String>,
}

/// Wire shape of a bounding box, in normalised `[0, 1]`
/// coordinates. Mirrors [`UnitBoundingBox`] but carries the
/// `JsonSchema` derive the structured-output schema needs (the core's
/// [`UnitBoundingBox`] does not depend on `schemars`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnitBox {
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
