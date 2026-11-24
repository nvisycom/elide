//! A reviewer's edits, for tests that apply a reviewed report.

use elide::entity::LabelRef;
use elide::modality::text::{Text, TextLocation};
use elide::{Orchestrator, PartId, Report, Review};

/// A report holding one entity a reviewer included in `part` at `location`
/// under `label`: one detection missed, redacted like any detected one.
pub fn included(part: &PartId, label: LabelRef, location: TextLocation) -> Report {
    let mut report = Report::new();
    let mut review = Review::new(&report);
    review
        .part::<Text>(part)
        .expect("a text part")
        .by("reviewer")
        .include(label, location);
    Orchestrator::new()
        .review(&mut report, &review)
        .expect("an include is accepted");
    report
}
