//! The image modality's always-on metadata-scrub analyzer.
//!
//! An image carries two redactable surfaces. Its EXIF metadata is surfaced by
//! [`ExifRecognizer`](elide::recognition::exif::ExifRecognizer) as
//! `Entity<Metadata>` and erased — always, regardless of the caller's
//! [`Analyzer`](crate::analyzer::Analyzer), which configures only the pixel side.

use elide::prelude::*;
use elide::recognition::exif::ExifRecognizer;

/// An analyzer that surfaces the image's privacy-relevant EXIF fields.
pub(super) fn metadata_analyzer() -> Analyzer<Metadata> {
    Analyzer::new()
        .with_recognizer(ExifRecognizer)
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
}
