//! The image modality's detect-and-redact stages: an always-available EXIF
//! metadata scrub, and — when an OCR enricher is wired — a pixel-text redaction.
//!
//! An image carries two redactable surfaces. Its EXIF metadata is surfaced by
//! [`ExifRecognizer`](elide::recognition::exif::ExifRecognizer) as
//! `Entity<Metadata>` and erased. Its rendered text is read by the OCR enricher
//! into a layout the text recognizers scan, and matched regions are blacked out
//! of the pixels.

use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::exif::ExifRecognizer;

use crate::enricher::ImageEnricherHandle;
use crate::recognizer::RecognizerHandle;

/// An analyzer that surfaces the image's privacy-relevant EXIF fields.
pub(super) fn metadata_analyzer() -> Analyzer<Metadata> {
    Analyzer::new()
        .with_recognizer(ExifRecognizer)
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
}

/// A metadata policy that erases every detected field.
pub(super) fn metadata_anonymizer() -> Anonymizer<Metadata> {
    Anonymizer::new().with(Rule::fallback(Erase))
}

/// An analyzer that reads the image's text with `enricher` (OCR) and scans it
/// with the given `recognizers`. Consumes the enricher and each recognizer.
pub(super) fn pixel_analyzer(
    enricher: ImageEnricherHandle,
    recognizers: Vec<RecognizerHandle>,
) -> Analyzer<Image> {
    let mut analyzer = Analyzer::new().with_enricher(enricher.into_enricher());
    for recognizer in recognizers {
        analyzer = analyzer.with_recognizer(recognizer.into_recognizer::<Image>());
    }
    analyzer
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
}

/// A pixel policy that blacks out every detected region.
pub(super) fn pixel_anonymizer() -> Anonymizer<Image> {
    Anonymizer::new().with(Rule::fallback(Blackbox::default()))
}
