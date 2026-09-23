//! Language-claim types.
//!
//! A [`LanguageClaim`] pairs a [`LanguageTag`] with its [`source`] — a
//! caller assertion or a detector's scored guess — and the byte-offset range it
//! applies to when a detector reports per-region results. A detector (or the
//! caller) builds a `Vec<LanguageClaim>` for one text scan.
//!
//! [`source`]: LanguageClaim::source

use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::LanguageTag;
use crate::primitive::Confidence;

/// Where a [`LanguageClaim`] came from, and how much to trust it.
///
/// A caller [`Asserted`](Self::Asserted) language is ground truth — treated as
/// full [`Confidence`], so it outranks any detector guess. A
/// [`Detected`](Self::Detected) language carries the detector's own score.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LanguageSource {
    /// Asserted by the caller: authoritative, full confidence.
    Asserted,
    /// Produced by a detection backend, with the backend's confidence score.
    Detected(Confidence),
}

impl LanguageSource {
    /// How much to trust the claim: [`MAX`](Confidence::MAX) for a caller
    /// assertion (ground truth), the detector's score for a detection. This is
    /// the sort key that ranks a call's languages best-first, so an assertion
    /// wins over any detection and detections order by their scores.
    #[must_use]
    pub fn confidence(&self) -> Confidence {
        match self {
            Self::Asserted => Confidence::MAX,
            Self::Detected(confidence) => *confidence,
        }
    }
}

/// A single claim that a span of text is in a [`language`](Self::language).
///
/// Its [`source`](Self::source) records whether the caller asserted it or a
/// detector found it (with a score), and [`span`](Self::span) the byte range it
/// covers when a detector reports per-region results — a whole-text or asserted
/// claim leaves it `None`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LanguageClaim {
    /// The claimed language.
    pub language: LanguageTag,
    /// Where the claim came from and how much to trust it.
    pub source: LanguageSource,
    /// Byte-offset range this claim applies to, when known. `None` means the
    /// whole text.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Option<Range<usize>>"))]
    pub span: Option<Range<usize>>,
}

impl LanguageClaim {
    /// A claim a detection backend produced, at `confidence`.
    #[must_use]
    pub fn detected(language: LanguageTag, confidence: Confidence) -> Self {
        Self {
            language,
            source: LanguageSource::Detected(confidence),
            span: None,
        }
    }

    /// A claim the caller asserted (authoritative, full confidence).
    #[must_use]
    pub fn asserted(language: LanguageTag) -> Self {
        Self {
            language,
            source: LanguageSource::Asserted,
            span: None,
        }
    }

    /// Attach the byte-offset span this claim covers.
    #[must_use]
    pub fn with_span(mut self, span: Range<usize>) -> Self {
        self.span = Some(span);
        self
    }

    /// The claim's confidence: its detector score, or full for an assertion.
    #[must_use]
    pub fn confidence(&self) -> Confidence {
        self.source.confidence()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> LanguageTag {
        LanguageTag::parse(s).unwrap()
    }

    #[test]
    fn confidence_ranks_asserted_above_any_detection() {
        // A caller assertion is full confidence, so it sorts ahead of even a
        // high-confidence detection; detections order by their own scores. This
        // is the ordering the recognition context sorts a call's languages by.
        let mut claims = [
            LanguageClaim::detected(tag("fr"), Confidence::new(0.9).unwrap()),
            LanguageClaim::asserted(tag("de")),
            LanguageClaim::detected(tag("es"), Confidence::new(0.5).unwrap()),
        ];
        claims.sort_by(|a, b| b.confidence().get().total_cmp(&a.confidence().get()));
        let order: Vec<&str> = claims.iter().map(|c| c.language.primary_subtag()).collect();
        assert_eq!(order, ["de", "fr", "es"]);
    }

    #[test]
    fn an_assertion_is_full_confidence() {
        assert_eq!(
            LanguageClaim::asserted(tag("en")).confidence(),
            Confidence::MAX
        );
    }
}
