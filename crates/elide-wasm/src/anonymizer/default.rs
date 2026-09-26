//! The default per-modality redaction policies, used when a stage's
//! [`Anonymizer`](super::Anonymizer) has no rules of its own.

use elide::prelude::operators::*;
use elide::prelude::*;

/// A readable default text policy: a labelled token per common kind, a masked
/// tail for payment cards, and full erasure for anything else.
pub(crate) fn text() -> Anonymizer<Text> {
    common_text(Anonymizer::new())
}

/// The tabular default policy — the same shape as [`text`], over cells.
pub(crate) fn tabular() -> Anonymizer<Tabular> {
    common_text(Anonymizer::new())
}

/// A pixel policy that blacks out every detected region.
pub(crate) fn image_pixels() -> Anonymizer<Image> {
    Anonymizer::new().with(Rule::fallback(Blackbox::default()))
}

/// The always-on metadata policy: erase every detected EXIF field. Not
/// caller-configurable; paired with the internal metadata analyzer.
pub(crate) fn image_metadata() -> Anonymizer<Metadata> {
    Anonymizer::new().with(Rule::fallback(Erase))
}

/// A policy that silences every detected span.
pub(crate) fn audio() -> Anonymizer<Audio> {
    Anonymizer::new().with(Rule::fallback(Silence))
}

/// The shared text-shaped policy, over any modality the operators support.
fn common_text<M>(anonymizer: Anonymizer<M>) -> Anonymizer<M>
where
    M: Modality,
    Keep: Operator<M>,
    Replace: Operator<M>,
    Mask: Operator<M>,
    Erase: Operator<M>,
{
    anonymizer
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
