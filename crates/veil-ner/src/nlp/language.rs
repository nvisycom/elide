//! Language-detection result types.
//!
//! [`LanguageDetection`] pairs a [`LanguageTag`] with how it was
//! obtained ([`LanguageProvenance`]: detected by a backend, or asserted
//! by the caller), an optional confidence, and the [`LanguageSpan`]
//! byte-offset range it applies to when the detector reports per-region
//! results. [`LanguageDetections`] is the typed wrapper a detector
//! produces for one text scan, suitable for storage on the shared
//! artifact bundle.

use serde::{Deserialize, Serialize};
use veil_core::primitive::{Confidence, LanguageTag};

/// Provenance of a [`LanguageDetection`].
///
/// Lets consumers distinguish "the engine ran a detector and got this
/// answer" from "the caller asserted this language and bypassed
/// detection", without overloading `confidence: None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguageProvenance {
    /// Produced by a language-detection backend.
    Detected,
    /// Asserted by the caller, bypassing detection.
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
#[derive(Serialize, Deserialize)]
pub struct LanguageSpan {
    /// Byte offset of the span start in the original text.
    pub start: usize,
    /// Byte offset of the span end in the original text.
    pub end: usize,
}

/// A single language detection result.
///
/// Carries the detected language plus an optional confidence and an
/// optional byte-offset [`LanguageSpan`]. Backends that don't expose
/// confidence (or where confidence isn't meaningful) leave it as `None`;
/// single-language detectors that don't track per-region information
/// leave `span` as `None`.
///
/// The `provenance` field records whether this answer came from a real
/// detector run or was asserted by the caller; backends only ever
/// produce [`LanguageProvenance::Detected`], with `Asserted` reserved
/// for callers that bypass detection.
#[derive(Debug, Clone, PartialEq)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageDetection {
    /// The detected language.
    pub language: LanguageTag,
    /// Optional confidence score. `None` when the backend doesn't expose
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// How this language was obtained: detected or caller-asserted.
    pub provenance: LanguageProvenance,
    /// Byte-offset range this detection applies to, when the backend
    /// reports per-region detections. Single-language detectors that
    /// answer "the whole text is X" leave this `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

    /// A language asserted by the caller, bypassing detection.
    #[must_use]
    pub fn asserted(language: LanguageTag) -> Self {
        Self {
            language,
            confidence: None,
            provenance: LanguageProvenance::Asserted,
            span: None,
        }
    }
}

/// Languages a language-detection backend resolved for one text scan.
///
/// Newtype around `Vec<LanguageDetection>` so a typed-map artifact bundle
/// sees a distinct typed entry. Whole-document detections store one entry
/// with `span = None`; multi-language documents store one entry per
/// language with the byte range covered.
///
/// Producers construct one of these for the text they scan and insert it
/// on a [`veil_core::recognition::Artifacts`] map; consumers that care
/// about language fetch it by type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LanguageDetections(pub Vec<LanguageDetection>);

impl LanguageDetections {
    /// Construct from a list of detections.
    #[must_use]
    pub fn new(detections: Vec<LanguageDetection>) -> Self {
        Self(detections)
    }

    /// Borrow the underlying detections.
    #[must_use]
    pub fn as_slice(&self) -> &[LanguageDetection] {
        &self.0
    }

    /// The language covering the most bytes of the source text, breaking
    /// ties on detector confidence.
    ///
    /// Monolingual docs return the single detection; mixed-language docs
    /// return the largest-coverage span; caller-asserted languages (no
    /// `span`) are treated as covering the whole document and therefore
    /// win against any one region.
    ///
    /// Returns `None` iff the list is empty.
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

fn span_bytes(d: &LanguageDetection) -> usize {
    match d.span {
        Some(s) => s.end.saturating_sub(s.start),
        None => usize::MAX,
    }
}

fn confidence_key(d: &LanguageDetection) -> f32 {
    d.confidence.map(|c| c.get()).unwrap_or(f32::NEG_INFINITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en() -> LanguageTag {
        LanguageTag::parse("en").unwrap()
    }

    fn de() -> LanguageTag {
        LanguageTag::parse("de").unwrap()
    }

    #[test]
    fn detected_and_asserted_record_provenance() {
        let detected = LanguageDetection::detected(en(), Confidence::new(0.9));
        assert_eq!(detected.provenance, LanguageProvenance::Detected);
        assert_eq!(detected.confidence, Confidence::new(0.9));

        let asserted = LanguageDetection::asserted(en());
        assert_eq!(asserted.provenance, LanguageProvenance::Asserted);
        assert!(asserted.confidence.is_none());
    }

    #[test]
    fn dominant_prefers_largest_span() {
        let small = LanguageDetection {
            span: Some(LanguageSpan { start: 0, end: 5 }),
            ..LanguageDetection::detected(de(), Confidence::new(0.99))
        };
        let large = LanguageDetection {
            span: Some(LanguageSpan { start: 5, end: 40 }),
            ..LanguageDetection::detected(en(), Confidence::new(0.6))
        };
        let detections = LanguageDetections::new(vec![small, large]);
        // Larger byte coverage wins over higher confidence.
        assert_eq!(detections.dominant().unwrap().language, en());
    }

    #[test]
    fn dominant_treats_unspanned_as_whole_document() {
        let region = LanguageDetection {
            span: Some(LanguageSpan { start: 0, end: 100 }),
            ..LanguageDetection::detected(de(), Confidence::new(0.99))
        };
        let asserted = LanguageDetection::asserted(en());
        let detections = LanguageDetections::new(vec![region, asserted]);
        // A span-less (caller-asserted) detection covers the whole document
        // and wins against any single region.
        assert_eq!(detections.dominant().unwrap().language, en());
    }

    #[test]
    fn dominant_is_none_for_empty() {
        assert!(LanguageDetections::default().dominant().is_none());
    }
}
