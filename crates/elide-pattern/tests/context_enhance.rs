//! Enhancer in isolation: a context keyword near an entity lifts its
//! confidence and the enhancer reports the lift as a [`Boost`].
//!
//! The enhancer is modality-agnostic and report-only, it mutates the
//! entity's confidence and returns the boosts; recording the located
//! refinement event is `Enhancer::apply`'s job, exercised through the
//! recognizer.

use std::ops::Range;

use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextLocation};
use elide_core::primitive::Confidence;
use elide_pattern::context::matching::SubstringMatcher;
use elide_pattern::context::{BoostRule, EnhanceInput, Enhancer};

/// Build a single text entity found at `range` in the recognized text.
fn entity(label: &LabelRef, range: Range<usize>, score: f32) -> Entity<Text> {
    let confidence = Confidence::new(score).unwrap();
    let location = TextLocation::inline(range.clone());
    let event = AuditEvent::pattern(
        "test",
        confidence,
        location.clone(),
        PatternEvent::default(),
    );
    let mut entity = Entity::new(label.clone(), location, AuditLog::new(event));
    entity.recognized_range = Some(range);
    entity
}

#[test]
fn keyword_in_window_boosts_and_records_refinement() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::new(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    //          0         1         2
    //          0123456789012345678901234567890
    let text = "social security 123-45-6789";
    let mut entities = vec![entity(&ssn, 16..27, 0.5)]; // "123-45-6789"

    let boosts = enhancer.enhance(&mut entities, &EnhanceInput::new(text));

    // 0.5 + 0.35 boost = 0.85.
    assert_eq!(entities[0].confidence, Confidence::new(0.85).unwrap());
    // One boost, from the in-text window (no hint).
    assert_eq!(boosts.len(), 1);
    let boost = &boosts[0];
    assert_eq!(boost.entity_index, 0);
    assert_eq!(boost.keyword, "social security");
    assert!(boost.hint_index.is_none());
    // The in-text path captures the keyword's range, "social security" is
    // bytes 0..15, so the caller can resolve its location.
    assert_eq!(boost.keyword_range, Some(0..15));
}

#[test]
fn no_keyword_leaves_entity_untouched() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::new(ssn.clone(), ["social security"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    let text = "the number is 123-45-6789";
    let mut entities = vec![entity(&ssn, 14..25, 0.5)];

    let boosts = enhancer.enhance(&mut entities, &EnhanceInput::new(text));

    assert_eq!(entities[0].confidence, Confidence::new(0.5).unwrap());
    assert!(boosts.is_empty()); // nothing fired
}

#[test]
fn out_of_band_hint_boosts_via_hint_path() {
    let ssn = LabelRef::new("US_SSN");
    let rule = BoostRule::new(ssn.clone(), ["ssn"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    // No keyword in the text, but the column header is supplied as a hint.
    let text = "123-45-6789";
    let hints = ["ssn"];
    let mut entities = vec![entity(&ssn, 0..11, 0.5)];

    let boosts = enhancer.enhance(&mut entities, &EnhanceInput::new(text).with_hints(&hints));

    assert_eq!(entities[0].confidence, Confidence::new(0.85).unwrap());
    // The boost fired from the first (and only) hint.
    assert_eq!(boosts.len(), 1);
    assert_eq!(boosts[0].hint_index, Some(0));
    // A hint match carries no in-text keyword range; its location lives on
    // the hint, resolved by the caller.
    assert!(boosts[0].keyword_range.is_none());
}

#[test]
fn anchor_boosts_when_a_qualifying_neighbour_is_in_window() {
    // A `monetary_amount` is lifted when a `currency` entity at/above the floor
    // sits in its window — the entity-proximity signal, not a keyword.
    let currency = LabelRef::new("currency");
    let amount = LabelRef::new("monetary_amount");
    let rule = BoostRule::new(amount.clone(), Vec::<String>::new())
        .with_anchor(currency.clone(), Confidence::new(0.35).unwrap());
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    //          0         1
    //          012345678901
    let text = "5000 USD ok";
    let mut entities = vec![
        entity(&amount, 0..4, 0.1),   // "5000"
        entity(&currency, 5..8, 0.5), // "USD", already confident
    ];
    enhancer.enhance(&mut entities, &EnhanceInput::new(text));
    // The amount was lifted by the currency anchor; the currency is untouched.
    assert!(
        entities[0].confidence.get() > 0.1,
        "amount lifted: {:?}",
        entities[0].confidence
    );
    assert_eq!(entities[1].confidence, Confidence::new(0.5).unwrap());
}

#[test]
fn anchor_does_not_boost_below_the_confidence_floor() {
    // A weak, sub-floor currency (an ambiguous token that never cleared its own
    // gate) does not anchor a monetary amount.
    let currency = LabelRef::new("currency");
    let amount = LabelRef::new("monetary_amount");
    let rule = BoostRule::new(amount.clone(), Vec::<String>::new())
        .with_anchor(currency.clone(), Confidence::new(0.35).unwrap());
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    let text = "2010 ADA xx";
    let mut entities = vec![
        entity(&amount, 0..4, 0.1),   // "2010"
        entity(&currency, 5..8, 0.3), // "ADA", below the 0.35 floor
    ];
    enhancer.enhance(&mut entities, &EnhanceInput::new(text));
    assert_eq!(
        entities[0].confidence,
        Confidence::new(0.1).unwrap(),
        "amount not lifted"
    );
}

#[test]
fn anchors_chain_to_a_fixpoint() {
    // Mutual anchors with asymmetric floors: a weak number seeds a currency
    // (low floor), which then lifts the number (threshold floor). Both end up
    // boosted after the fixpoint.
    let currency = LabelRef::new("currency");
    let amount = LabelRef::new("monetary_amount");
    let amount_rule = BoostRule::new(amount.clone(), Vec::<String>::new())
        .with_anchor(currency.clone(), Confidence::new(0.35).unwrap());
    let currency_rule = BoostRule::new(currency.clone(), Vec::<String>::new())
        .with_anchor(amount.clone(), Confidence::new(0.1).unwrap());
    let enhancer = Enhancer::new([amount_rule, currency_rule], SubstringMatcher);

    let text = "5000 USD ok";
    let mut entities = vec![
        entity(&amount, 0..4, 0.1),   // weak number
        entity(&currency, 5..8, 0.3), // weak code, below amount's floor initially
    ];
    enhancer.enhance(&mut entities, &EnhanceInput::new(text));
    // Round 1: number (0.1 ≥ 0.1) lifts currency → 0.65. Round 2: currency
    // (0.65 ≥ 0.35) lifts number. Both above their starting confidence.
    assert!(
        entities[1].confidence.get() > 0.3,
        "currency lifted: {:?}",
        entities[1].confidence
    );
    assert!(
        entities[0].confidence.get() > 0.1,
        "amount lifted: {:?}",
        entities[0].confidence
    );
}

#[test]
fn term_scoped_anchor_applies_only_to_listed_terms() {
    // A number-anchor restricted to the currency-codes terms lifts "USD" (in the
    // set) but not "Polygon" (a currency-labeled crypto name not in the set),
    // even though both carry the `currency` label.
    let currency = LabelRef::new("currency");
    let amount = LabelRef::new("monetary_amount");
    let rule = BoostRule::new(currency.clone(), Vec::<String>::new())
        .with_anchor(amount.clone(), Confidence::new(0.1).unwrap())
        .with_self_terms(["USD", "EUR"]);
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    // "USD" is a listed code → lifted by the nearby amount.
    let text = "5000 USD ok";
    let mut e = vec![entity(&amount, 0..4, 0.1), entity(&currency, 5..8, 0.3)];
    enhancer.enhance(&mut e, &EnhanceInput::new(text));
    assert!(
        e[1].confidence.get() > 0.3,
        "listed code lifted: {:?}",
        e[1].confidence
    );

    // "Polygon" is not in the set → not lifted, despite the same label + a number.
    let text = "2 Polygon xx";
    let mut e = vec![entity(&amount, 0..1, 0.1), entity(&currency, 2..9, 0.3)];
    enhancer.enhance(&mut e, &EnhanceInput::new(text));
    assert_eq!(
        e[1].confidence,
        Confidence::new(0.3).unwrap(),
        "unlisted term not lifted"
    );
}

#[test]
fn anchor_self_exclusion_survives_a_rangeless_preceding_entity() {
    // Regression for the filtered-snapshot index fix: a rangeless entity before
    // the real ones shifts the filtered snapshot's positions relative to the
    // full `entities` slice. Self-exclusion must compare original indices, or
    // the genuine currency neighbour would be mistaken for the amount itself and
    // skipped, leaving the amount un-boosted.
    let currency = LabelRef::new("currency");
    let amount = LabelRef::new("monetary_amount");
    let rule = BoostRule::new(amount.clone(), Vec::<String>::new())
        .with_anchor(currency.clone(), Confidence::new(0.35).unwrap());
    let enhancer = Enhancer::new([rule], SubstringMatcher);

    //          0         1
    //          0123456789012
    let text = "ref 5000 USD";
    // A rangeless entity first (e.g. a natively-located box), then the amount and
    // the currency. Its presence used to misalign the self-exclusion index.
    let mut rangeless = entity(&LabelRef::new("other"), 0..3, 0.5);
    rangeless.recognized_range = None;
    let mut entities = vec![
        rangeless,
        entity(&amount, 4..8, 0.1),    // "5000"
        entity(&currency, 9..12, 0.5), // "USD"
    ];
    enhancer.enhance(&mut entities, &EnhanceInput::new(text));
    // The amount is lifted by the currency neighbour despite the leading
    // rangeless entity.
    assert!(
        entities[1].confidence.get() > 0.1,
        "amount lifted past the rangeless entity: {:?}",
        entities[1].confidence
    );
}
