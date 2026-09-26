//! The text modality's detect-and-redact stage: an analyzer folded from the
//! caller's recognizers, and a readable default redaction policy.

use elide::enrichment::lingua::LinguaEnricher;
use elide::prelude::operators::*;
use elide::prelude::*;

use crate::recognizer::RecognizerHandle;

/// An analyzer over the given recognizers (and optional language enricher), with
/// the reconcile-and-filter layers every modality shares. Consumes each handle.
pub(super) fn analyzer(
    recognizers: Vec<RecognizerHandle>,
    language: Option<LinguaEnricher>,
) -> Analyzer<Text> {
    let mut analyzer = Analyzer::new();
    if let Some(language) = language {
        analyzer = analyzer.with_enricher(language);
    }
    for recognizer in recognizers {
        analyzer = analyzer.with_recognizer(recognizer.into_text());
    }
    analyzer
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
}

/// A readable default redaction policy: a labelled token per common kind, a
/// masked tail for payment cards, and full erasure for anything else.
pub(super) fn anonymizer() -> Anonymizer<Text> {
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
