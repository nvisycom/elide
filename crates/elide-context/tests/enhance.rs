//! Enhancer in isolation: a context keyword near a draft lifts its
//! confidence and the enhancer reports the lift as a [`Boost`].
//!
//! The enhancer is modality-agnostic and report-only — it mutates the
//! draft's confidence and returns the boosts; recording the located
//! refinement event is the `Enhanced` adapter's job, exercised separately.

use std::ops::Range;

use elide_context::matching::SubstringMatcher;
use elide_context::{BoostRule, Context, Enhancer};
use elide_core::entity::LabelRef;
use elide_core::recognition::{DraftEvent, EntityDraft};
use elide_core::entity::provenance::PatternEvent;
use elide_core::primitive::Confidence;

/// Build a single draft at `range` in the stream.
fn draft(label: &LabelRef, range: Range<usize>, score: f32) -> EntityDraft {
    let event = DraftEvent::pattern("test", "test", PatternEvent::default());
    EntityDraft::new(label.clone(), Confidence::new(score).unwrap(), range, event)
}

#[test]
fn keyword_in_window_boosts_and_records_refinement() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    //          0         1         2
    //          0123456789012345678901234567890
    let text = "social security 123-45-6789";
    let mut drafts = vec![draft(&ssn, 16..27, 0.5)]; // "123-45-6789"

    let boosts = enhancer.enhance(&mut drafts, &Context::new(text));

    // 0.5 + 0.35 boost = 0.85.
    assert_eq!(drafts[0].confidence, Confidence::new(0.85).unwrap());
    // One boost, from the in-text window (no hint).
    assert_eq!(boosts.len(), 1);
    let boost = &boosts[0];
    assert_eq!(boost.entity_index, 0);
    assert_eq!(boost.keyword, "social security");
    assert!(boost.hint_index.is_none());
    // The in-text path captures the keyword's *stream* range — "social
    // security" is bytes 0..15 — so the caller can resolve its location.
    assert_eq!(boost.keyword_range, Some(0..15));
}

#[test]
fn no_keyword_leaves_draft_untouched() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    let text = "the number is 123-45-6789";
    let mut drafts = vec![draft(&ssn, 14..25, 0.5)];

    let boosts = enhancer.enhance(&mut drafts, &Context::new(text));

    assert_eq!(drafts[0].confidence, Confidence::new(0.5).unwrap());
    assert!(boosts.is_empty()); // nothing fired
}

#[test]
fn out_of_band_hint_boosts_via_hint_path() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["ssn"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    // No keyword in the text, but the column header is supplied as a hint.
    let text = "123-45-6789";
    let hints = ["ssn"];
    let mut drafts = vec![draft(&ssn, 0..11, 0.5)];

    let boosts = enhancer.enhance(&mut drafts, &Context::new(text).with_hints(&hints));

    assert_eq!(drafts[0].confidence, Confidence::new(0.85).unwrap());
    // The boost fired from the first (and only) hint.
    assert_eq!(boosts.len(), 1);
    assert_eq!(boosts[0].hint_index, Some(0));
    // A hint match carries no in-text keyword range; its location lives on
    // the hint, resolved by the caller.
    assert!(boosts[0].keyword_range.is_none());
}
