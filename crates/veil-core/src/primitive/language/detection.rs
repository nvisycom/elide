//! Language-detection result types.
//!
//! [`LanguageDetection`] pairs a [`LanguageTag`] with how it was obtained
//! ([`LanguageProvenance`]: detected by a backend, or asserted by the
//! caller), an optional confidence, and the [`LanguageSpan`] byte-offset
//! range it applies to when the detector reports per-region results.
//! [`LanguageDetections`] is the list a detector (or the caller) builds
//! for one text scan.

use std::cmp::Ordering;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::LanguageTag;
use crate::primitive::Confidence;

/// How a [`LanguageDetection`]'s language was obtained.
///
/// Lets consumers distinguish "a detector ran and got this answer" from
/// "the caller asserted this language". An assertion may still carry an
/// optional confidence, so this is independent of the confidence field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub enum LanguageProvenance {
    /// Produced by a language-detection backend.
    Detected,
    /// Asserted by the caller.
    Asserted,
}

/// A byte-offset range within the analyzed text.
///
/// Attached to a [`LanguageDetection`] when the detector knows the span
/// its answer covers (mixed-language input produces multiple detections,
/// each with a distinct span). Single-language detections from
/// non-segmenting backends, and caller-asserted answers, typically leave
/// the span as `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LanguageSpan {
    /// Byte offset of the span start in the original text.
    pub start: usize,
    /// Byte offset of the span end in the original text.
    pub end: usize,
}

/// A single language detection result.
///
/// Carries the language plus an optional confidence and an optional
/// byte-offset [`LanguageSpan`]. Backends that don't expose confidence
/// leave it `None`; single-language detectors that don't track per-region
/// information leave `span` as `None`. The `provenance` field records
/// whether the answer came from a detector or was asserted by the caller.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct LanguageDetection {
    /// The language.
    pub language: LanguageTag,
    /// Optional confidence score. `None` when not exposed.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub confidence: Option<Confidence>,
    /// How this language was obtained: detected or caller-asserted.
    pub provenance: LanguageProvenance,
    /// Byte-offset range this detection applies to, when known. `None`
    /// means the whole text.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub span: Option<LanguageSpan>,
}

impl LanguageDetection {
    /// A language produced by a detection backend, with optional
    /// confidence.
    #[must_use]
    pub fn detected(language: LanguageTag, confidence: Option<Confidence>) -> Self {
        Self {
            language,
            confidence,
            provenance: LanguageProvenance::Detected,
            span: None,
        }
    }

    /// A language asserted by the caller, with optional confidence.
    #[must_use]
    pub fn asserted(language: LanguageTag, confidence: Option<Confidence>) -> Self {
        Self {
            language,
            confidence,
            provenance: LanguageProvenance::Asserted,
            span: None,
        }
    }

    /// Attach a byte-offset span this detection covers.
    #[must_use]
    pub fn with_span(mut self, span: LanguageSpan) -> Self {
        self.span = Some(span);
        self
    }

    /// Rank against another for "best language" ordering.
    ///
    /// [`Greater`](Ordering::Greater) is the stronger candidate: higher
    /// confidence wins (a missing confidence ranks below any present one),
    /// and at equal confidence an [`Asserted`](LanguageProvenance::Asserted)
    /// language beats a [`Detected`](LanguageProvenance::Detected) one.
    fn rank(&self, other: &Self) -> Ordering {
        confidence_key(self)
            .total_cmp(&confidence_key(other))
            .then_with(|| provenance_rank(self).cmp(&provenance_rank(other)))
    }
}

/// A list of [`LanguageDetection`]s resolved for one text scan.
///
/// Built by a detector (one entry per detected region) or by the caller
/// asserting languages. Carried on a [`RecognizerInput`] so every
/// recognizer and the context enhancer can consult the call's languages.
///
/// [`RecognizerInput`]: crate::recognition::RecognizerInput
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct LanguageDetections(pub Vec<LanguageDetection>);

impl LanguageDetections {
    /// Construct from a list of detections.
    #[must_use]
    pub fn new(detections: Vec<LanguageDetection>) -> Self {
        Self(detections)
    }

    /// Add a detection to the list.
    pub fn push(&mut self, detection: LanguageDetection) {
        self.0.push(detection);
    }

    /// Borrow the detections in their stored order.
    #[must_use]
    pub fn as_slice(&self) -> &[LanguageDetection] {
        &self.0
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The detections ranked best-first: by confidence descending (a
    /// missing confidence sorts last), with an asserted language breaking
    /// ties ahead of a detected one. The sort is stable.
    #[must_use]
    pub fn ranked(&self) -> Vec<&LanguageDetection> {
        let mut out: Vec<&LanguageDetection> = self.0.iter().collect();
        out.sort_by(|a, b| b.rank(a));
        out
    }

    /// The single best language, or `None` when the list is empty.
    #[must_use]
    pub fn best(&self) -> Option<&LanguageDetection> {
        self.0.iter().max_by(|a, b| a.rank(b))
    }

    /// The language covering the most bytes of the source text, breaking
    /// ties on confidence.
    ///
    /// Caller-asserted or whole-document detections (no `span`) are
    /// treated as covering the whole text, so they win against any single
    /// region. Returns `None` iff the list is empty.
    #[must_use]
    pub fn dominant(&self) -> Option<&LanguageDetection> {
        self.0.iter().max_by(|a, b| {
            span_bytes(a)
                .cmp(&span_bytes(b))
                .then_with(|| confidence_key(a).total_cmp(&confidence_key(b)))
        })
    }
}

impl From<Vec<LanguageDetection>> for LanguageDetections {
    fn from(detections: Vec<LanguageDetection>) -> Self {
        Self::new(detections)
    }
}

/// Confidence as a sort key: present scores by value, a missing score as
/// negative infinity so it ranks below any present one.
fn confidence_key(d: &LanguageDetection) -> f32 {
    d.confidence
        .map(Confidence::get)
        .unwrap_or(f32::NEG_INFINITY)
}

/// Provenance tiebreak: a higher number wins, so an assertion outranks a
/// detection at equal confidence.
fn provenance_rank(d: &LanguageDetection) -> u8 {
    match d.provenance {
        LanguageProvenance::Asserted => 1,
        LanguageProvenance::Detected => 0,
    }
}

/// Byte coverage of a detection, treating a missing span as the whole
/// document (maximal coverage).
fn span_bytes(d: &LanguageDetection) -> usize {
    match d.span {
        Some(s) => s.end.saturating_sub(s.start),
        None => usize::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> LanguageTag {
        LanguageTag::parse(s).unwrap()
    }

    #[test]
    fn ranked_orders_by_confidence_then_assertion() {
        let dets = LanguageDetections::new(vec![
            LanguageDetection::detected(tag("fr"), Confidence::new(0.8)),
            LanguageDetection::asserted(tag("de"), None),
            LanguageDetection::detected(tag("es"), None),
            LanguageDetection::asserted(tag("it"), Confidence::new(0.8)),
        ]);
        let order: Vec<&str> = dets
            .ranked()
            .iter()
            .map(|d| d.language.primary_language())
            .collect();
        // 0.8 scores first; among them asserted (it) beats detected (fr).
        // Then the None-confidence pair; asserted (de) beats detected (es).
        assert_eq!(order, ["it", "fr", "de", "es"]);
    }

    #[test]
    fn best_is_top_of_ranked() {
        let dets = LanguageDetections::new(vec![
            LanguageDetection::detected(tag("fr"), Confidence::new(0.8)),
            LanguageDetection::asserted(tag("de"), None),
        ]);
        // Confidence-first: detected French (0.8) beats asserted German (None).
        assert_eq!(dets.best().unwrap().language, tag("fr"));
    }

    #[test]
    fn dominant_prefers_largest_span() {
        let small = LanguageDetection::detected(tag("de"), Confidence::new(0.99))
            .with_span(LanguageSpan { start: 0, end: 5 });
        let large = LanguageDetection::detected(tag("en"), Confidence::new(0.6))
            .with_span(LanguageSpan { start: 5, end: 40 });
        let dets = LanguageDetections::new(vec![small, large]);
        assert_eq!(dets.dominant().unwrap().language, tag("en"));
    }

    #[test]
    fn empty_has_no_best_or_dominant() {
        let dets = LanguageDetections::default();
        assert!(dets.is_empty());
        assert!(dets.best().is_none());
        assert!(dets.dominant().is_none());
    }
}
