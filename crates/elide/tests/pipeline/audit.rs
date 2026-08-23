//! Each entity carries its own tamper-evident audit trail (an [`AuditLog`]),
//! reachable through the facade and verifiable with no `serde` feature.

use elide::entity::audit::{AuditEvent, AuditKind, AuditLog, PatternEvent};
use elide::entity::{Entity, LabelRef};
use elide::modality::text::{Text, TextLocation};
use elide::primitive::Confidence;

/// A text entity found by a pattern, then redacted, so its trail has two
/// linked events.
fn redacted_entity(label: &str) -> Entity<Text> {
    use elide::entity::audit::RuleMatch;
    use elide::redaction::{LeakProfile, OperatorId};

    let location = TextLocation::new(0, 5);
    let conf = Confidence::new(0.9).unwrap();
    let mut audit = AuditLog::new(AuditEvent::pattern(
        "t",
        conf,
        location.clone(),
        PatternEvent::default(),
    ));
    audit.record(AuditEvent::redaction(
        OperatorId::new("erase", "1.0.0"),
        LeakProfile::Irrecoverable,
        conf,
        RuleMatch::Fallback,
        None,
    ));
    Entity::new(LabelRef::new(label), location, conf, audit)
}

#[test]
fn entity_audit_trail_is_reachable_and_verifies() {
    let entity = redacted_entity("EMAIL_ADDRESS");

    // The trail records both events, linked, and verifies.
    assert_eq!(entity.audit.events().len(), 2);
    assert!(matches!(
        entity.audit.events()[0].kind,
        AuditKind::Pattern(_)
    ));
    assert!(matches!(
        entity.audit.events()[1].kind,
        AuditKind::Redaction(_)
    ));
    // The redaction links to the detection (its single parent).
    assert_eq!(entity.audit.events()[1].parents().len(), 1);
    assert_eq!(
        entity.audit.events()[1].parents()[0],
        entity.audit.events()[0].hash()
    );
    assert!(entity.audit.verify().is_ok());
}

/// Tamper-evidence over the *stored* form: serialize the trail, edit the JSON,
/// deserialize, and re-verify. The recomputed hash no longer matches.
#[cfg(feature = "serde")]
#[test]
fn tampering_with_the_serialized_trail_breaks_verification() {
    let entity = redacted_entity("EMAIL_ADDRESS");
    let mut value = serde_json::to_value(&entity.audit).unwrap();

    // Rewrite the detected label's confidence in the stored trail, leaving its
    // stored hash intact.
    value[0]["confidence"] = serde_json::json!(0.1);
    let tampered: AuditLog<Text> = serde_json::from_value(value).unwrap();

    assert!(tampered.verify().is_err());
}

/// Appending a validly-chained event past the trail's end is caught: the trail
/// must have a single tip, and a second self-consistent tail node leaves two
/// sinks.
#[cfg(feature = "serde")]
#[test]
fn appending_a_valid_tail_event_breaks_verification() {
    let entity = redacted_entity("EMAIL_ADDRESS");
    let value = serde_json::to_value(&entity.audit).unwrap();
    let events = value.as_array().unwrap();

    // Re-append the (self-consistent) last event: it still chains from a real
    // parent and its own hash checks out, so a per-event check passes, but now
    // two events are sinks.
    let mut forged = events.clone();
    forged.push(events.last().unwrap().clone());
    let tampered: AuditLog<Text> = serde_json::from_value(serde_json::json!(forged)).unwrap();

    assert!(tampered.verify().is_err());
}

/// Splicing a self-consistent birth (parentless) event from *another* trail
/// into the middle is caught: its hash is referenced by nothing here, so it is
/// an extra sink.
#[cfg(feature = "serde")]
#[test]
fn inserting_an_orphan_event_breaks_verification() {
    let victim = serde_json::to_value(&redacted_entity("EMAIL_ADDRESS").audit).unwrap();
    // A different entity's birth event: distinct hash, parentless, valid on its
    // own, but unrelated to the victim trail.
    let foreign = serde_json::to_value(&redacted_entity("PHONE_NUMBER").audit).unwrap();
    let foreign_birth = foreign.as_array().unwrap()[0].clone();

    let mut forged = victim.as_array().unwrap().clone();
    forged.insert(1, foreign_birth);
    let tampered: AuditLog<Text> = serde_json::from_value(serde_json::json!(forged)).unwrap();

    assert!(tampered.verify().is_err());
}
