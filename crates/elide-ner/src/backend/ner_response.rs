//! [`NerResponse`] and [`NerSpan`]: what a [`NerBackend`] returns.
//!
//! A backend emits [`NerSpan`]s already carrying a canonical
//! [`LabelRef`]: a zero-shot model is *given* the catalog labels and
//! echoes them, and a fixed-label model maps its raw output to canonical
//! labels itself (the [`LabelMap`] utility in `elide-core` helps).
//! The recognizer then only filters and scores; it does no label
//! translation.
//!
//! Score is the model's confidence in the raw `[0.0, 1.0]` range; the
//! recognizer clamps to [`Confidence`] and may demote configured
//! low-score labels.
//!
//! [`NerBackend`]: super::NerBackend
//! [`Confidence`]: elide_core::primitive::Confidence
//! [`LabelMap`]: elide_core::recognition::LabelMap

use std::ops::Range;

use elide_core::entity::LabelRef;
use elide_core::primitive::Confidence;

/// One per-call NER response from a [`NerBackend`].
///
/// Wraps the spans the backend produced. Each span already carries a
/// canonical [`LabelRef`]; the recognizer applies its ignore-set before
/// emitting entities.
///
/// [`NerBackend`]: super::NerBackend
#[derive(Debug, Clone, Default)]
pub struct NerResponse {
    /// Spans predicted for the request's text, in backend order.
    pub spans: Vec<NerSpan>,
}

impl NerResponse {
    /// Construct a response from spans.
    #[must_use]
    pub fn new(spans: Vec<NerSpan>) -> Self {
        Self { spans }
    }
}

/// One entity span predicted by a NER model.
///
/// The label is a canonical [`LabelRef`], not a raw model string: the
/// backend is responsible for any raw-to-canonical translation (see the
/// [`LabelMap`] utility). Coordinate space is byte offsets into the source
/// text the backend was called with.
///
/// [`LabelMap`]: elide_core::recognition::LabelMap
#[derive(Debug, Clone, PartialEq)]
pub struct NerSpan {
    /// Canonical entity label for this span.
    pub label: LabelRef,
    /// Model confidence for this span; a backend clamps its raw score into
    /// range when constructing the span.
    pub confidence: Confidence,
    /// Byte range of the prediction in the source text.
    pub offset: Range<usize>,
}

impl NerSpan {
    /// Construct a span from a canonical label name and a raw score,
    /// clamping the score into `[0, 1]`.
    pub fn new(label: impl Into<String>, score: f32, offset: Range<usize>) -> Self {
        Self {
            label: LabelRef::new(label.into()),
            confidence: Confidence::clamped(score),
            offset,
        }
    }

    /// Construct a span from an existing [`LabelRef`] and [`Confidence`].
    pub fn with_label(label: LabelRef, confidence: Confidence, offset: Range<usize>) -> Self {
        Self {
            label,
            confidence,
            offset,
        }
    }
}
