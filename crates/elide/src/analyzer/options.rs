//! [`AnalysisOptions`]: per-call inputs for [`Analyzer::analyze`] and
//! [`Analyzer::analyze_stream`].
//!
//! [`Analyzer::analyze`]: super::Analyzer::analyze
//! [`Analyzer::analyze_stream`]: super::Analyzer::analyze_stream

use derive_builder::Builder;
use elide_core::modality::Modality;
use elide_core::primitive::{CountryCode, Languages};
use elide_core::recognition::{Hint, RecognizerInput};

/// Per-call options shared across every chunk of one analysis.
///
/// Bundles the per-call concerns the caller asserts (the document's
/// languages, its jurisdictions, document-level labels, annotation hints)
/// so they can be passed once and merged into each chunk's
/// [`RecognizerInput`]. The modality payload is *not* here: it comes from
/// the content or the streamed source.
///
/// Build with [`AnalysisOptions::builder`]; every field defaults to
/// empty, so default options assert nothing and every recognizer runs
/// with its own defaults.
#[derive(Debug, Builder)]
#[builder(
    name = "AnalysisOptionsBuilder",
    pattern = "owned",
    setter(into, prefix = "with"),
    default
)]
pub struct AnalysisOptions<M: Modality> {
    /// Caller-asserted languages for the call.
    languages: Languages,
    /// Caller-asserted jurisdictions. A recognizer rule scoped to some
    /// countries runs when any asserted country matches.
    countries: Vec<CountryCode>,
    /// Document-level classification labels (e.g. `"medical"`).
    labels: Vec<String>,
    /// Caller-supplied annotation regions.
    hints: Vec<Hint<M>>,
}

// Manual `Clone` / `Default`: `derive` would add spurious `M: Clone` /
// `M: Default` bounds, but `M` is a zero-size marker. The fields satisfy
// both on their own.
impl<M: Modality> Clone for AnalysisOptions<M> {
    fn clone(&self) -> Self {
        Self {
            languages: self.languages.clone(),
            countries: self.countries.clone(),
            labels: self.labels.clone(),
            hints: self.hints.clone(),
        }
    }
}

impl<M: Modality> Default for AnalysisOptions<M> {
    fn default() -> Self {
        Self {
            languages: Languages::default(),
            countries: Vec::new(),
            labels: Vec::new(),
            hints: Vec::new(),
        }
    }
}

impl<M: Modality> AnalysisOptions<M> {
    /// Empty options: nothing asserted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a builder for per-call options.
    #[must_use]
    pub fn builder() -> AnalysisOptionsBuilder<M> {
        AnalysisOptionsBuilder::default()
    }

    /// Merge these options onto `input`, returning the enriched input.
    ///
    /// Used internally to fold the per-call options into the
    /// [`RecognizerInput`] built from each chunk's payload.
    pub(super) fn apply_to(&self, mut input: RecognizerInput<M>) -> RecognizerInput<M> {
        if !self.languages.is_empty() {
            input.languages = self.languages.clone();
        }
        if !self.countries.is_empty() {
            input = input.with_countries(self.countries.clone());
        }
        if !self.labels.is_empty() {
            input = input.with_labels(self.labels.clone());
        }
        if !self.hints.is_empty() {
            input = input.with_hints(self.hints.clone());
        }
        input
    }
}
