//! Language-detection result types.
//!
//! [`Language`] pairs a [`LanguageTag`] with how it was obtained
//! ([`LanguageProvenance`]: detected by a backend, or asserted by the
//! caller), an optional confidence, and the byte-offset range it applies to
//! when the detector reports per-region results. A detector (or the caller)
//! builds a `Vec<Language>` for one text scan.

use std::cmp::Ordering;
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::LanguageTag;
use crate::primitive::Confidence;

/// How a [`Language`]'s language was obtained.
///
/// Lets consumers distinguish "a detector ran and got this answer" from
/// "the caller asserted this language". An assertion may still carry an
/// optional confidence, so this is independent of the confidence field.
///
/// The variants are declared weakest-first, so the derived [`Ord`] makes an
/// [`Asserted`](Self::Asserted) provenance outrank a
/// [`Detected`](Self::Detected) one — the tiebreak a language ranking applies at
/// equal confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LanguageProvenance {
    /// Produced by a language-detection backend.
    Detected,
    /// Asserted by the caller.
    Asserted,
}

/// Single language detection result.
///
/// Carries the language plus an optional confidence and an optional
/// byte-offset range. Backends that don't expose confidence
/// leave it `None`; single-language detectors that don't track per-region
/// information leave `span` as `None`. The `provenance` field records
/// whether the answer came from a detector or was asserted by the caller.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Language {
    /// Language.
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
    #[cfg_attr(feature = "schema", schemars(with = "Option<Range<usize>>"))]
    pub span: Option<Range<usize>>,
}

impl Language {
    /// Language produced by a detection backend.
    ///
    /// Attach a score with [`with_confidence`].
    ///
    /// [`with_confidence`]: Self::with_confidence
    #[must_use]
    pub fn detected(language: LanguageTag) -> Self {
        Self {
            language,
            confidence: None,
            provenance: LanguageProvenance::Detected,
            span: None,
        }
    }

    /// Language asserted by the caller.
    ///
    /// Attach a score with [`with_confidence`].
    ///
    /// [`with_confidence`]: Self::with_confidence
    #[must_use]
    pub fn asserted(language: LanguageTag) -> Self {
        Self {
            language,
            confidence: None,
            provenance: LanguageProvenance::Asserted,
            span: None,
        }
    }

    /// Attach a confidence score.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Attach a byte-offset span this detection covers.
    #[must_use]
    pub fn with_span(mut self, span: Range<usize>) -> Self {
        self.span = Some(span);
        self
    }

    /// Rank against another for "best language" ordering.
    ///
    /// [`Greater`] is the stronger candidate: higher
    /// confidence wins (a missing confidence ranks below any present one),
    /// and at equal confidence an [`Asserted`]
    /// language beats a [`Detected`] one.
    ///
    /// [`Greater`]: Ordering::Greater
    /// [`Asserted`]: LanguageProvenance::Asserted
    /// [`Detected`]: LanguageProvenance::Detected
    pub(crate) fn rank(&self, other: &Self) -> Ordering {
        self.confidence_key()
            .total_cmp(&other.confidence_key())
            .then_with(|| self.provenance.cmp(&other.provenance))
    }

    /// Confidence as a sort key: the score when present, and negative infinity
    /// when absent so an unscored language ranks below any scored one.
    fn confidence_key(&self) -> f32 {
        self.confidence
            .map(Confidence::get)
            .unwrap_or(f32::NEG_INFINITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> LanguageTag {
        LanguageTag::parse(s).unwrap()
    }

    #[test]
    fn rank_orders_by_confidence_then_assertion() {
        // `rank` is the ordering the recognition context sorts a call's
        // languages by (best-first): higher confidence wins, and an asserted
        // language breaks a tie ahead of a detected one.
        let mut langs = [
            Language::detected(tag("fr")).with_confidence(Confidence::new(0.8).unwrap()),
            Language::asserted(tag("de")),
            Language::detected(tag("es")),
            Language::asserted(tag("it")).with_confidence(Confidence::new(0.8).unwrap()),
        ];
        langs.sort_by(|a, b| b.rank(a));
        let order: Vec<&str> = langs
            .iter()
            .map(|d| d.language.primary_language())
            .collect();
        // 0.8 scores first; among them asserted (it) beats detected (fr).
        // Then the None-confidence pair; asserted (de) beats detected (es).
        assert_eq!(order, ["it", "fr", "de", "es"]);
    }

    #[test]
    fn rank_puts_confidence_before_provenance() {
        // Confidence-first: detected French (0.8) outranks asserted German
        // (no confidence).
        let fr = Language::detected(tag("fr")).with_confidence(Confidence::new(0.8).unwrap());
        let de = Language::asserted(tag("de"));
        assert_eq!(fr.rank(&de), Ordering::Greater);
    }
}
