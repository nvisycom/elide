//! The image modality's always-on metadata-scrub analyzer.
//!
//! An image carries two redactable surfaces. Its EXIF metadata is surfaced by
//! [`FieldRecognizer`](elide::recognition::metadata::FieldRecognizer) as
//! `Entity<Metadata>` and erased — always, regardless of the caller's
//! [`Analyzer`](crate::analyzer::Analyzer), which configures only the pixel side.

use elide::prelude::*;
use elide::recognition::metadata::FieldRecognizer;

/// An analyzer that surfaces the image's privacy-relevant EXIF fields.
pub(super) fn metadata_analyzer() -> Analyzer<Metadata> {
    Analyzer::new()
        .with_recognizer(FieldRecognizer)
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
}
