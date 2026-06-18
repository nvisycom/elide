//! Enhancer end-to-end: a context keyword near an entity lifts its
//! confidence and records a refinement event.

use elide_context::matching::SubstringMatcher;
use elide_context::{BoostRule, Context, Enhancer};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextLocation};
use elide_core::primitive::Confidence;
use elide_core::provenance::{Event, EventKind, PatternEvent, Provenance};

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

    enhancer.enhance(&mut entities, &Context::new(text));

    let e = &entities[0];
    // 0.5 + 0.35 boost = 0.85.
    assert_eq!(e.confidence, Confidence::new(0.85).unwrap());
    // The trail now has the recognition event + a refinement event.
    assert_eq!(e.provenance.events.len(), 2);
    let last = e.provenance.events.last().unwrap();
    assert!(matches!(
        last.kind,
        EventKind::Refinement { ref keyword, in_hint: false } if keyword == "social security"
    ));
}

#[test]
fn no_keyword_leaves_entity_untouched() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    let text = "the number is 123-45-6789";
    let mut entities = vec![entity(&ssn, 14, 25, 0.5)];

    enhancer.enhance(&mut entities, &Context::new(text));

    let e = &entities[0];
    assert_eq!(e.confidence, Confidence::new(0.5).unwrap());
    assert_eq!(e.provenance.events.len(), 1); // recognition only
}

#[test]
fn out_of_band_hint_boosts_via_hint_path() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::for_label(ssn.clone(), ["ssn"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    // No keyword in the text, but the column header is supplied as a hint.
    let text = "123-45-6789";
    let hints = vec!["ssn".to_string()];
    let mut entities = vec![entity(&ssn, 0, 11, 0.5)];

    enhancer.enhance(&mut entities, &Context::new(text).with_hints(&hints));

    let e = &entities[0];
    assert_eq!(e.confidence, Confidence::new(0.85).unwrap());
    let last = e.provenance.events.last().unwrap();
    assert!(matches!(
        last.kind,
        EventKind::Refinement { in_hint: true, .. }
    ));
}
