//! Enhancer in isolation: a context keyword near an entity lifts its
//! confidence and the enhancer reports the lift as a [`Boost`].
//!
//! The enhancer is modality-agnostic and report-only — it mutates the
//! entity's confidence and returns the boosts; recording the located
//! refinement event is the `Enhanced` adapter's job, exercised separately.

use std::ops::Range;

use elide_context::matching::SubstringMatcher;
use elide_context::{BoostRule, Context, Enhancer};
use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextLocation};
use elide_core::primitive::Confidence;

/// Build a single text entity found at `range` in the recognized text.
fn entity(label: &LabelRef, range: Range<usize>, score: f32) -> Entity<Text> {
    let confidence = Confidence::new(score).unwrap();
    let location = TextLocation::new(range.start, range.end);
    let event = Event::pattern("test", confidence, location.clone(), PatternEvent::default());
    let mut entity = Entity::new(label.clone(), location, confidence, Provenance::new(event));
    entity.recognized_range = Some(range);
    entity
}

#[test]
fn keyword_in_window_boosts_and_records_refinement() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    //          0         1         2
    //          0123456789012345678901234567890
    let text = "social security 123-45-6789";
    let mut entities = vec![entity(&ssn, 16..27, 0.5)]; // "123-45-6789"

    let boosts = enhancer.enhance(&mut entities, &Context::new(text));

    // 0.5 + 0.35 boost = 0.85.
    assert_eq!(entities[0].confidence, Confidence::new(0.85).unwrap());
    // One boost, from the in-text window (no hint).
    assert_eq!(boosts.len(), 1);
    let boost = &boosts[0];
    assert_eq!(boost.entity_index, 0);
    assert_eq!(boost.keyword, "social security");
    assert!(boost.hint_index.is_none());
    // The in-text path captures the keyword's range — "social security" is
    // bytes 0..15 — so the caller can resolve its location.
    assert_eq!(boost.keyword_range, Some(0..15));
}

#[test]
fn no_keyword_leaves_entity_untouched() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    let text = "the number is 123-45-6789";
    let mut entities = vec![entity(&ssn, 14..25, 0.5)];

    let boosts = enhancer.enhance(&mut entities, &Context::new(text));

    assert_eq!(entities[0].confidence, Confidence::new(0.5).unwrap());
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
    let mut entities = vec![entity(&ssn, 0..11, 0.5)];

    let boosts = enhancer.enhance(&mut entities, &Context::new(text).with_hints(&hints));

    assert_eq!(entities[0].confidence, Confidence::new(0.85).unwrap());
    // The boost fired from the first (and only) hint.
    assert_eq!(boosts.len(), 1);
    assert_eq!(boosts[0].hint_index, Some(0));
    // A hint match carries no in-text keyword range; its location lives on
    // the hint, resolved by the caller.
    assert!(boosts[0].keyword_range.is_none());
}
