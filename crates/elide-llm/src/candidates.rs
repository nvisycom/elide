//! Structured-output candidate types the model is asked to produce.
//!
//! [`Candidates<C>`] is the `T` in rig's `Extractor::<T>`, the backend
//! asks the model to fill it in, and the [`JsonSchema`] derive constrains
//! the output. The item type `C` is the per-modality candidate
//! ([`TextCandidate`] / [`ImageCandidate`]); the recognizer localizes each
//! candidate into the source and builds the final entity.

use elide_core::primitive::UnitBoundingBox;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The model's `{"entities": [...]}` structured-output response.
///
/// Generic over the per-modality candidate item `C` ([`TextCandidate`] /
/// [`ImageCandidate`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Candidates<C> {
    /// Detected candidates.
    pub entities: Vec<C>,
}

// Hand-written so the bound is none, an empty batch needs no `C: Default`,
// which a derive would spuriously require.
impl<C> Default for Candidates<C> {
    fn default() -> Self {
        Self {
            entities: Vec::new(),
        }
    }
}

/// Wire shape of one text entity the model detected in the source.
///
/// A surface form (`value`) plus the surrounding `context` that locates it
/// back into a byte range in the source text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextCandidate {
    /// Matched text value: the literal surface form to flag.
    pub value: String,
    /// Brief description of the real-world entity (advisory).
    #[serde(default)]
    pub description: Option<String>,
    /// Canonical label naming the entity's type.
    pub label: String,
    /// Model-asserted confidence in `[0.0, 1.0]`.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Short surrounding text intended to uniquely locate `value`.
    /// Missing or non-unique `context` causes the candidate to be dropped
    /// per the recognizer's policy.
    #[serde(default)]
    pub context: Option<String>,
    /// Coreference identifier shared across mentions of one real-world
    /// entity within a call.
    #[serde(default)]
    pub coreference: Option<String>,
}

/// Wire shape of one image entity the model detected in the source.
///
/// A region (its normalised bounding box) plus a label and an optional
/// description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImageCandidate {
    /// Region's normalised bounding box: the image candidate's value.
    #[serde(flatten)]
    pub bbox: UnitBox,
    /// Brief description of the region (advisory).
    #[serde(default)]
    pub description: Option<String>,
    /// Canonical label naming the region's type.
    pub label: String,
    /// Model-asserted confidence in `[0.0, 1.0]`.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Coreference identifier shared across mentions of one real-world
    /// entity within a call.
    #[serde(default)]
    pub coreference: Option<String>,
}

/// Wire shape of a normalised `[0, 1]` bounding box, as two corners.
///
/// Top-left (`x_min`, `y_min`) + bottom-right (`x_max`, `y_max`).
///
/// Two-corner (`xyxy`) rather than corner+size (`xywh`): it matches the
/// native output of vision models that ground boxes (Gemini emits
/// `[ymin, xmin, ymax, xmax]`) and the document-OCR convention (Google
/// Document AI, Azure, AWS Textract all report corner vertices), so the
/// model is asked for the shape it already produces. A corner-based DTO
/// distinct from [`UnitBoundingBox`]'s origin-plus-size shape;
/// [`to_unit_bounding_box`](Self::to_unit_bounding_box) converts to the box
/// the rest of the pipeline uses.
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

impl UnitBox {
    /// The region this box covers as a [`UnitBoundingBox`], or `None` when it
    /// covers none.
    ///
    /// A model's box is taken for the region it means rather than the field
    /// order it used: each pair of edges is ordered, so swapped corners still
    /// cover their region, and each edge is clamped into the image. A
    /// non-finite edge, or a box with no width or height, covers nothing.
    #[must_use]
    pub fn to_unit_bounding_box(self) -> Option<UnitBoundingBox> {
        let (x_min, x_max) = edges(self.x_min, self.x_max)?;
        let (y_min, y_max) = edges(self.y_min, self.y_max)?;
        let (width, height) = (x_max - x_min, y_max - y_min);
        (width > 0.0 && height > 0.0).then(|| UnitBoundingBox::new(x_min, y_min, width, height))
    }
}

/// Two edges of one axis, ordered and clamped into `0.0..=1.0`, or `None`
/// when either is not finite.
fn edges(a: f64, b: f64) -> Option<(f64, f64)> {
    if !a.is_finite() || !b.is_finite() {
        return None;
    }
    Some((a.min(b).clamp(0.0, 1.0), a.max(b).clamp(0.0, 1.0)))
}
