//! Enhancer in isolation: a context keyword near an entity lifts its
//! confidence and the enhancer reports the lift as a [`Boost`].
//!
//! The enhancer is modality-agnostic and report-only — it mutates
//! confidence and returns the boosts; recording the located refinement
//! event is the M-aware wrapper's job, exercised separately.

use elide_context::matching::SubstringMatcher;
use elide_context::{BoostRule, Context, Enhancer};
use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextLocation};
use elide_core::primitive::Confidence;

/// Build a single-recognition entity at `start..end`.
fn entity(label: &LabelRef, start: usize, end: usize, score: f32) -> Entity<Text> {
    let location = TextLocation::new(start, end);
    let confidence = Confidence::new(score).unwrap();
    let event = Event::pattern(
        "test",
        confidence,
        location.clone(),
        PatternEvent::default(),
    );
    Entity::new(label.clone(), location, confidence, Provenance::new(event))
}

#[test]
fn keyword_in_window_boosts_and_records_refinement() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    //          0         1         2
    //          0123456789012345678901234567890
    let text = "social security 123-45-6789";
    let mut entities = vec![entity(&ssn, 16, 27, 0.5)]; // "123-45-6789"

    let boosts = enhancer.enhance(&mut entities, &Context::new(text));

    let e = &entities[0];
    // 0.5 + 0.35 boost = 0.85.
    assert_eq!(e.confidence, Confidence::new(0.85).unwrap());
    // One boost, from the in-text window (no hint).
    assert_eq!(boosts.len(), 1);
    let boost = &boosts[0];
    assert_eq!(boost.entity_index, 0);
    assert_eq!(boost.keyword, "social security");
    assert!(boost.hint_index.is_none());
}

#[test]
fn no_keyword_leaves_entity_untouched() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    let text = "the number is 123-45-6789";
    let mut entities = vec![entity(&ssn, 14, 25, 0.5)];

    let boosts = enhancer.enhance(&mut entities, &Context::new(text));

    let e = &entities[0];
    assert_eq!(e.confidence, Confidence::new(0.5).unwrap());
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
    let mut entities = vec![entity(&ssn, 0, 11, 0.5)];

    let boosts = enhancer.enhance(&mut entities, &Context::new(text).with_hints(&hints));

    let e = &entities[0];
    assert_eq!(e.confidence, Confidence::new(0.85).unwrap());
    // The boost fired from the first (and only) hint.
    assert_eq!(boosts.len(), 1);
    assert_eq!(boosts[0].hint_index, Some(0));
}
