//! Structured-output candidate types: the typed schema the model is asked
//! to produce.
//!
//! [`Candidates<C>`] is the `T` in rig's `Extractor::<T>` — the backend
//! asks the model to fill it in, and the [`JsonSchema`] derive constrains
//! the output. The item type `C` is the per-modality candidate
//! ([`TextCandidate`] / [`ImageCandidate`]); the recognizer localizes each
//! candidate into the source and builds the final entity.

use elide_core::primitive::UnitBoundingBox;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The model's `{"entities": [...]}` response, generic over the
/// per-modality candidate item `C`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Candidates<C> {
    /// Detected candidates.
    pub entities: Vec<C>,
}

// Hand-written so the bound is none — an empty batch needs no `C: Default`,
// which a derive would spuriously require.
impl<C> Default for Candidates<C> {
    fn default() -> Self {
        Self {
            entities: Vec::new(),
        }
    }
}

/// One entity candidate produced by the model for the text modality.
///
/// Carries the surface form (`value`) plus a surrounding `context` snippet
/// the recognizer uses to localize the value back into a byte range in the
/// source text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextCandidate {
    /// Model-assigned identifier for the underlying real-world entity.
    /// Stable across coreferent mentions within one call.
    #[serde(default)]
    pub entity_id: Option<String>,
    /// Label name. Missing (`None`) means the model declined to type the
    /// candidate; the recognizer drops these.
    pub entity_type: Option<String>,
    /// The matched text value: the literal surface form to flag.
    pub value: String,
    /// Model-asserted confidence in `[0.0, 1.0]`.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Short surrounding text intended to uniquely locate `value`.
    /// Missing or non-unique `context` causes the candidate to be dropped
    /// per the recognizer's policy.
    #[serde(default)]
    pub context: Option<String>,
    /// Brief description of the real-world entity (advisory).
    #[serde(default)]
    pub description: Option<String>,
}

/// One image entity discovered by the model. Bounding box is normalised
/// (`[0, 1]`); the recognizer scales to pixel coordinates using the source
/// image's dimensions.
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

/// Wire shape of a bounding box, in normalised `[0, 1]` coordinates as two
/// corners (top-left + bottom-right).
///
/// Two-corner (`xyxy`) rather than corner+size (`xywh`): it matches the
/// native output of vision models that ground boxes (Gemini emits
/// `[ymin, xmin, ymax, xmax]`) and the document-OCR convention (Google
/// Document AI, Azure, AWS Textract all report corner vertices), so the
/// model is asked for the shape it already produces. Mirrors
/// [`UnitBoundingBox`] but carries the `JsonSchema` derive the
/// structured-output schema needs (core's [`UnitBoundingBox`] does not
/// depend on `schemars`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnitBox {
    /// Left edge x in `0.0..=1.0` (fraction of image width).
    pub x_min: f64,
    /// Top edge y in `0.0..=1.0` (fraction of image height).
    pub y_min: f64,
    /// Right edge x in `0.0..=1.0` (fraction of image width).
    pub x_max: f64,
    /// Bottom edge y in `0.0..=1.0` (fraction of image height).
    pub y_max: f64,
}

impl From<UnitBox> for UnitBoundingBox {
    fn from(b: UnitBox) -> Self {
        let width = (b.x_max - b.x_min).max(0.0);
        let height = (b.y_max - b.y_min).max(0.0);
        UnitBoundingBox::new(b.x_min, b.y_min, width, height)
    }
}
