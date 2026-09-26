//! The tabular modality's detect-and-redact stage. `Tabular` is
//! [`TextRecognizable`](elide::modality::TextRecognizable), so the same text
//! recognizers scan each cell.

use elide::enrichment::lingua::LinguaEnricher;
use elide::prelude::operators::*;
use elide::prelude::*;

use crate::recognizer::RecognizerHandle;

/// An analyzer over the given recognizers (and optional language enricher), with
/// the shared reconcile-and-filter layers. Consumes each handle.
pub(super) fn analyzer(
    recognizers: Vec<RecognizerHandle>,
    language: Option<LinguaEnricher>,
) -> Analyzer<Tabular> {
    let mut analyzer = Analyzer::new();
    if let Some(language) = language {
        analyzer = analyzer.with_enricher(language);
    }
    for recognizer in recognizers {
        analyzer = analyzer.with_recognizer(recognizer.into_recognizer::<Tabular>());
    }
    analyzer
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
}

/// A readable default cell-redaction policy: a labelled token per common kind, a
/// masked tail for payment cards, and full erasure for anything else.
pub(super) fn anonymizer() -> Anonymizer<Tabular> {
    Anonymizer::new()
        .with(Rule::predicate(
            |cx| !ConfidenceThreshold::BASELINE.passes(cx.entity.confidence),
            Keep,
        ))
        .with(Rule::label(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[EMAIL]"),
        ))
        .with(Rule::label(
            builtins::PHONE_NUMBER.to_ref(),
            Replace::new("[PHONE]"),
        ))
        .with(Rule::label(builtins::URL.to_ref(), Replace::new("[URL]")))
        .with(Rule::label(
            builtins::PAYMENT_CARD.to_ref(),
            Mask::stars().with_keep_suffix(4),
        ))
        .with(Rule::fallback(Erase))
}
