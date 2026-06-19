//! Assembles the redaction side of the pipeline: an [`Anonymizer`] that
//! maps a redaction operator to each label.

use elide::Anonymizer;
use elide::entity::builtins;
use elide::modality::text::Text;
use elide::primitive::ConfidenceThreshold;
use elide::redaction::operators::{Erase, Keep, Mask, Replace};

/// Build an anonymizer that picks a redaction strategy per label.
pub fn build_anonymizer() -> Anonymizer<Text> {
    Anonymizer::new()
        // A weak detection (below the baseline threshold) is kept as-is,
        // before any label rule can fire. Order matters: the first
        // matching rule wins.
        .with_predicate(
            |e| !ConfidenceThreshold::BASELINE.passes(e.confidence),
            Keep,
        )
        .with_label(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]"))
        .with_label(builtins::PHONE_NUMBER.to_ref(), Replace::new("[PHONE]"))
        .with_label(builtins::URL.to_ref(), Replace::new("[URL]"))
        // Keep the last four digits of a card visible, mask the rest.
        .with_label(
            builtins::PAYMENT_CARD.to_ref(),
            Mask::stars().with_keep_suffix(4),
        )
        // Anything else we detect gets fully removed.
        .with_fallback(Erase)
}
