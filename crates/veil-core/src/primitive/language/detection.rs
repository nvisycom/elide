//! The detected (or asserted) language of a piece of content.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::LanguageTag;
use crate::primitive::Confidence;

/// How a [`LanguageDetection`]'s language was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LanguageProvenance {
    /// Produced by a language-detection backend.
    Detected,
    /// Asserted by the caller, bypassing detection.
    Asserted,
}

/// The language of a piece of content, with how it was determined.
///
/// Either a detection backend identified the language (with an optional
/// confidence), or the caller asserted it outright — the [`provenance`]
/// field records which. Recognizers scoped to a language consult this to
/// decide whether to run.
///
/// [`provenance`]: Self::provenance
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LanguageDetection {
    /// The detected language.
    pub language: LanguageTag,
    /// Optional confidence score. `None` when the backend doesn't expose
    /// one.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub confidence: Option<Confidence>,
    /// How this language was obtained: detected or caller-asserted.
    pub provenance: LanguageProvenance,
}

impl LanguageDetection {
    /// A language produced by a detection backend, with optional confidence.
    pub fn detected(language: LanguageTag, confidence: Option<Confidence>) -> Self {
        Self {
            language,
            confidence,
            provenance: LanguageProvenance::Detected,
        }
    }

    /// A language asserted by the caller, bypassing detection.
    pub fn asserted(language: LanguageTag) -> Self {
        Self {
            language,
            confidence: None,
            provenance: LanguageProvenance::Asserted,
        }
    }
}
