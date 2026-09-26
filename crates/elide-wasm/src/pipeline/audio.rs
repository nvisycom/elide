//! The audio modality's detect-and-redact stage.
//!
//! Detection needs a transcript, which the browser supplies through an STT
//! enricher; with one wired the recognizers scan the transcript and matched time
//! spans are silenced. Without it, an audio clip round-trips unchanged.

use elide::enrichment::stt::SttEnricher;
use elide::prelude::operators::*;
use elide::prelude::*;

use crate::recognizer::RecognizerHandle;

/// An analyzer whose optional STT `enricher` produces the transcript the given
/// recognizers scan, with the shared reconcile-and-filter layers. Consumes each
/// handle.
pub(super) fn analyzer(
    recognizers: Vec<RecognizerHandle>,
    enricher: Option<SttEnricher>,
) -> Analyzer<Audio> {
    let mut analyzer = Analyzer::new();
    if let Some(enricher) = enricher {
        analyzer = analyzer.with_enricher(enricher);
    }
    for recognizer in recognizers {
        analyzer = analyzer.with_recognizer(recognizer.into_recognizer::<Audio>());
    }
    analyzer
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
}

/// A policy that silences every detected span.
pub(super) fn anonymizer() -> Anonymizer<Audio> {
    Anonymizer::new().with(Rule::fallback(Silence))
}
