//! The redaction side of the demo pipeline.

use elide::prelude::operators::*;
use elide::prelude::*;

/// A redaction policy: a readable token per common label, a masked tail for
/// payment cards, and full erasure for anything else detected.
pub(crate) fn build_anonymizer() -> Anonymizer<Text> {
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
