//! The redaction side of the pipeline: a policy wrapped as a handle.

use elide::prelude::operators::*;
use elide::prelude::*;
use wasm_bindgen::prelude::*;

/// A redaction policy. Borrowed by [`redact`](crate::redact) and reusable.
#[wasm_bindgen]
pub struct AnonymizerHandle(Anonymizer<Text>);

impl AnonymizerHandle {
    /// The wrapped anonymizer, for the redact entry point.
    pub(crate) fn anonymizer(&self) -> &Anonymizer<Text> {
        &self.0
    }
}

/// Build the demo's redaction policy: a readable token per common label, a
/// masked tail for payment cards, and full erasure for anything else detected.
#[wasm_bindgen(js_name = createAnonymizer)]
pub fn create_anonymizer() -> AnonymizerHandle {
    let anonymizer = Anonymizer::new()
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
        .with(Rule::fallback(Erase));
    AnonymizerHandle(anonymizer)
}
