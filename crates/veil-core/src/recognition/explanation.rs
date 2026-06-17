//! The [`Explanation`] audit record behind a single detection.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::primitive::Confidence;

/// The reasoning behind a single [`Detection`].
///
/// This is the per-detection audit record, modelled on Presidio's
/// `AnalysisExplanation` but **always present** (Presidio strips it
/// unless decision logging is enabled) and enriched. It captures *why*
/// one detection layer believed it saw an entity, in enough detail to
/// reconstruct or contest the decision after the fact.
///
/// Not every field applies to every recognizer: a regex pattern
/// recognizer fills [`pattern`](Self::pattern) and
/// [`validation`](Self::validation); a model/NER recognizer fills
/// [`textual`](Self::textual). Context-based score boosting populates
/// [`context_words`](Self::context_words) and the
/// [`original_confidence`](Self::original_confidence)/effective-score delta.
///
/// [`Detection`]: crate::recognition::Detection
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Explanation {
    /// The raw confidence before any context enhancement.
    pub original_confidence: Option<Confidence>,
    /// Name of the matched pattern, for pattern recognizers.
    pub pattern_name: Option<HipStr<'static>>,
    /// The literal pattern (e.g. regex source) that matched.
    pub pattern: Option<HipStr<'static>>,
    /// Outcome of a post-match validator (e.g. Luhn checksum):
    /// `Some(true)` validated, `Some(false)` invalidated, `None` if no
    /// validator ran.
    pub validation: Option<bool>,
    /// Context words in the surrounding text that boosted the score.
    pub context_words: Vec<HipStr<'static>>,
    /// Free-text explanation, typically from model/NER recognizers.
    pub textual: Option<HipStr<'static>>,
}

impl Explanation {
    /// An empty explanation, to be filled in by the recognizer.
    pub fn new() -> Self {
        Self::default()
    }
}
