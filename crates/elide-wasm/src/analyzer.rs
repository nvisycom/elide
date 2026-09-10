//! The detection side of the demo pipeline.

use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

/// The pattern-recognizer analyzer plus its deduplication pipeline. `patterns`
/// and `dictionaries` toggle the two recognizer sources so the demo can show
/// what each contributes; with both off the analyzer detects nothing.
///
/// # Errors
///
/// Propagates a build error from the pattern recognizer (an invalid shipped
/// rule), which cannot happen with the built-in set.
pub(crate) fn build_analyzer(patterns: bool, dictionaries: bool) -> Result<Analyzer<Text>> {
    let mut builder = PatternRecognizer::builder();
    if patterns {
        builder = builder.with_builtin_patterns();
    }
    if dictionaries {
        builder = builder.with_builtin_dictionaries();
    }
    let recognizer = builder.build_context_enhanced()?;

    Ok(Analyzer::new()
        .with_recognizer(recognizer)
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE)))
}
