//! End-to-end shape test: two recognizers find the same entity, a
//! fusion combines them into one provenanced entity — exercising the
//! modality-generic model and the provenance ledger the way the toolkit
//! fusion step would.

use elide_core::Result;
use elide_core::entity::{Entity, EntityCoRef, Label, LabelCatalog, LabelRef};
use elide_core::modality::Modality;
use elide_core::primitive::{Confidence, ConfidenceThreshold, CountryCode, LanguageTag};
use elide_core::provenance::{Event, EventKind, Manifest, ModelEvent, PatternEvent, Provenance};

mod fixtures;
use fixtures::{Text, TextData, TextLocation, TextReplacement};

/// Build a single-recognition entity, the way a recognizer would.
fn recognized(
    label: &LabelRef,
    location: TextLocation,
    confidence: Confidence,
    event: Event<Text>,
) -> Entity<Text> {
    Entity::new(label.clone(), location, confidence, Provenance::new(event))
}

/// A trivial "highest confidence wins" fusion: concatenate every
/// entity's events and append a deduplication event — what the toolkit
/// fusion step does, assembled here by hand from core parts.
fn fuse_max_confidence(mut entities: Vec<Entity<Text>>) -> Entity<Text> {
    entities.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    let mut base = entities.remove(0);
    let before = base.confidence;
    for other in entities {
        base.provenance.events.extend(other.provenance.events);
    }
    base.provenance
        .record(Event::deduplication("max", before, base.confidence));
    base
}

#[test]
fn two_recognizers_fuse_into_one() {
    let phone = LabelRef::new("PHONE_NUMBER");

    // Recognizer 1: a regex pattern.
    let pattern_conf = Confidence::new(0.8).unwrap();
    let pattern = recognized(
        &phone,
        TextLocation::new(10, 22),
        pattern_conf,
        Event::pattern(
            "us-phone-pattern",
            pattern_conf,
            TextLocation::new(10, 22),
            PatternEvent {
                name: "phone".into(),
                regex: Some("\\d{3}-\\d{3}-\\d{4}".into()),
                validator: Some("luhn".into()),
                contextual: false,
            },
        ),
    );

    // Recognizer 2: an NER model, slightly different span, higher confidence.
    let ner_conf = Confidence::new(0.95).unwrap();
    let ner = recognized(
        &phone,
        TextLocation::new(10, 23),
        ner_conf,
        Event::model(
            "ner-model",
            ner_conf,
            TextLocation::new(10, 23),
            ModelEvent {
                name: "ner-model".into(),
                version: Some("2024.1".into()),
                contextual: false,
            },
        ),
    );

    // Fuse both into one provenanced entity.
    let mut entity = fuse_max_confidence(vec![pattern, ner]);

    // The fusion kept the highest-confidence layer's location and score.
    assert_eq!(entity.label, phone);
    assert_eq!(entity.confidence, Confidence::new(0.95).unwrap());

    // The entity has a fresh v7 identity and a matching reference.
    assert_eq!(entity.id.get_version_num(), 7);
    assert_eq!(entity.as_ref().id(), entity.id);

    // Coreference is unset by default; it can be attached.
    assert!(entity.coref.is_none());
    entity.coref = Some(EntityCoRef::new("ref-1"));
    assert_eq!(
        entity.coref.as_ref().map(EntityCoRef::as_str),
        Some("ref-1")
    );

    // Both recognitions survive, plus a deduplication event.
    assert_eq!(entity.provenance.recognizers().count(), 2);
    assert_eq!(entity.provenance.events.len(), 3);
    assert!(matches!(
        entity.provenance.events.last().unwrap().kind,
        EventKind::Deduplication { ref strategy } if strategy == "max"
    ));
    assert_eq!(
        entity.provenance.final_confidence(),
        Some(Confidence::new(0.95).unwrap())
    );
}

#[test]
fn label_catalog_resolves_refs() {
    let catalog: LabelCatalog = [
        Label::described("PHONE_NUMBER", "A telephone number"),
        Label::new("EMAIL_ADDRESS"),
    ]
    .into_iter()
    .collect();

    let phone = LabelRef::new("PHONE_NUMBER");
    assert_eq!(
        catalog.get(&phone).and_then(Label::description),
        Some("A telephone number")
    );
    assert!(catalog.contains(&LabelRef::new("EMAIL_ADDRESS")));
    assert!(!catalog.contains(&LabelRef::new("SSN")));

    // Modality name is a type-level constant.
    assert_eq!(<Text as Modality>::NAME, "text");
}

#[test]
fn threshold_filters_by_confidence() {
    let cutoff = ConfidenceThreshold::BASELINE;
    assert_eq!(cutoff.get(), 0.35);
    assert!(cutoff.passes(Confidence::new(0.95).unwrap()));
    assert!(!cutoff.passes(Confidence::new(0.2).unwrap()));

    // Out-of-range construction returns None.
    assert!(Confidence::new(1.5).is_none());
    assert!(ConfidenceThreshold::new(-0.1).is_none());
}

#[test]
fn manifest_anchors_a_run() {
    let manifest = Manifest::new("e3b0c44298fc1c14", "0.1.0");
    assert_eq!(manifest.version.as_str(), "0.1.0");
    // UUIDv7 is time-ordered and non-nil.
    assert!(!manifest.run_id.is_nil());
}

#[test]
fn language_tag_parses_and_exposes_subtags() {
    let tag = LanguageTag::parse("en-US").unwrap();
    assert_eq!(tag.primary_language(), "en");
    assert_eq!(tag.region(), Some("US"));
    assert_eq!(tag.as_str(), "en-US");

    // Malformed tags are rejected.
    assert!(LanguageTag::parse("not a tag!").is_err());
}

#[test]
fn country_code_resolves_iso_codes() {
    let us = CountryCode::from_alpha2("US").unwrap();
    assert_eq!(us.alpha3(), "USA");
    assert_eq!(us.to_string(), "US");
    assert_eq!(CountryCode::from_alpha3("USA").unwrap(), us);

    // Unknown codes are rejected.
    assert!(CountryCode::from_alpha2("ZZ").is_err());
}

#[test]
fn geometry_shapes_compose() {
    use elide_core::primitive::geometry::{BoundingBox, Point, Polygon};

    let bbox = BoundingBox::from_origin_size(Point::new(10.0, 20.0), 100.0, 40.0);
    assert_eq!(bbox.width(), 100.0);
    assert_eq!(bbox.height(), 40.0);
    assert_eq!(bbox.max, Point::new(110.0, 60.0));

    let poly: Polygon = [
        Point::new(0.0, 0.0),
        Point::new(1.0, 0.0),
        Point::new(0.0, 1.0),
    ]
    .into_iter()
    .collect();
    assert_eq!(poly.len(), 3);
}

#[test]
fn label_map_translates_raw_labels() {
    use elide_core::recognition::LabelMap;

    let map: LabelMap = [
        ("PER", LabelRef::new("PERSON")),
        ("LOC", LabelRef::new("LOCATION")),
    ]
    .into_iter()
    .collect();

    assert_eq!(map.get("PER"), Some(&LabelRef::new("PERSON")));
    assert!(map.contains("LOC"));
    assert!(map.get("ORG").is_none());
}

#[test]
fn recognizer_context_scopes_by_language_and_country() {
    use elide_core::recognition::{RecognizerContext, RecognizerLanguage};

    let en_us = LanguageTag::parse("en-US").unwrap();
    let en = LanguageTag::parse("en").unwrap();
    let fr = LanguageTag::parse("fr").unwrap();

    // Primary-subtag matching: "en" matches "en-US".
    assert!(en.matches(&en_us));
    assert!(!en.matches(&fr));

    let ctx: RecognizerContext<Text> = RecognizerContext::new()
        .with_language(en_us.clone(), None)
        .with_country(CountryCode::from_alpha2("US").unwrap());

    // The asserted language is the primary one.
    assert_eq!(ctx.primary_language(), Some(&en_us));

    // Empty scope always applies.
    assert!(ctx.applies_to_language(&[]));
    assert!(ctx.applies_to_country(&[]));
    // Matching scope applies; non-matching does not.
    assert!(ctx.applies_to_language(&[en]));
    assert!(!ctx.applies_to_language(&[fr]));
    assert!(ctx.applies_to_country(&[CountryCode::from_alpha2("US").unwrap()]));
    assert!(!ctx.applies_to_country(&[CountryCode::from_alpha2("GB").unwrap()]));
}

#[test]
fn recognizer_context_carries_hints() {
    use elide_core::entity::LabelRef;
    use elide_core::modality::text::TextLocation;
    use elide_core::recognition::{Hint, RecognizerContext};

    let hint = Hint::new(TextLocation::new(0, 5))
        .with_name("uploaded selection")
        .with_label(LabelRef::new("PERSON"));
    let ctx: RecognizerContext<Text> = RecognizerContext::new().with_hints(vec![hint]);

    assert_eq!(ctx.hints.len(), 1);
    assert_eq!(ctx.hints[0].location, TextLocation::new(0, 5));
    assert_eq!(ctx.hints[0].name.as_deref(), Some("uploaded selection"));
    assert_eq!(ctx.hints[0].label, Some(LabelRef::new("PERSON")));
}

#[test]
fn operator_trait_shape() {
    use elide_core::redaction::{LeakProfile, Operator, OperatorId};

    /// A trivial `[LABEL]`-style replace operator, to exercise the
    /// trait shape and the pure `Replacement` model.
    struct Replace;

    impl Operator<Text> for Replace {
        fn id(&self) -> OperatorId {
            OperatorId::new("replace", "1.0.0")
        }

        fn leak_profile(&self) -> LeakProfile {
            LeakProfile::Partial
        }

        async fn anonymize(
            &self,
            entity: &elide_core::entity::Entity<Text>,
            _data: &TextData,
        ) -> Result<TextReplacement> {
            // Pure: computes the replacement, mutates nothing.
            Ok(TextReplacement::Substituted(format!(
                "[{}]",
                entity.label.as_str()
            )))
        }
    }

    let op = Replace;
    assert_eq!(op.id(), OperatorId::new("replace", "1.0.0"));
    assert_eq!(op.leak_profile(), LeakProfile::Partial);
    assert!(LeakProfile::Recoverable < LeakProfile::Irrecoverable);

    // Both replacement variants are constructible.
    let _ = TextReplacement::Substituted("[PHONE_NUMBER]".into());
    let _ = TextReplacement::Removed;
}
