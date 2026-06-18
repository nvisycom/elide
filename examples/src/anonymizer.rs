//! Assembles the redaction side of the pipeline: an [`Anonymizer`] that
//! maps a redaction operator to each label.

use elide::entity::builtins;
use elide::modality::text::Text;
use elide::redaction::Anonymizer;
use elide::redaction::operators::{Mask, Redact, Replace};

/// Build an anonymizer that picks a redaction strategy per label.
pub fn build_anonymizer() -> Anonymizer<Text> {
    Anonymizer::new()
        .with_operator(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]"))
        .with_operator(builtins::PHONE_NUMBER.to_ref(), Replace::new("[PHONE]"))
        .with_operator(builtins::URL.to_ref(), Replace::new("[URL]"))
        // Keep the last four digits of a card visible, mask the rest.
        .with_operator(
            builtins::PAYMENT_CARD.to_ref(),
            Mask::stars().with_keep_suffix(4),
        )
        // Anything else we detect gets fully removed.
        .with_fallback(Redact)
}
