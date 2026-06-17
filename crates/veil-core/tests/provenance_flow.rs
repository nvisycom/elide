//! End-to-end shape test: two recognizers find the same entity, a
//! fusion combines them into one provenanced entity — exercising the
//! modality-generic model and the provenance ledger the way the toolkit
//! fusion step would.

use veil_core::Result;
use veil_core::entity::{Entity, EntityCoRef, Label, LabelCatalog, LabelRef};
use veil_core::modality::Modality;
use veil_core::primitive::{Confidence, ConfidenceThreshold, CountryCode, LanguageTag};
use veil_core::provenance::{Manifest, Provenance};
use veil_core::recognition::{Detection, Explanation, Merge, RecognizerId};

mod fixtures;
use fixtures::{Text, TextData, TextLocation, TextReplacement};

/// A trivial "highest confidence wins" fusion, of the kind the toolkit
/// fusion step will provide — assembled here by hand from core parts.
fn fuse_max_confidence(detections: Vec<Detection<Text>>) -> Entity<Text> {
    let top = detections
        .iter()
        .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
        .expect("non-empty");
    let label = top.label.clone();
    let location = top.location.clone();
    let confidence = top.confidence;
    let merge = Merge::new("max", confidence);
    Entity::new(
        label,
        location,
        confidence,
        Provenance::merged(detections, merge),
    )
}

#[test]
fn two_recognizers_fuse_into_one() {
    let phone = LabelRef::new("PHONE_NUMBER");

    // Recognizer 1: a regex pattern.
    let pattern = Detection::new(
        RecognizerId::new("us-phone-pattern", "1.0.0"),
        phone.clone(),
        TextLocation::new(10, 22),
        Confidence::new(0.8).unwrap(),
        Explanation {
            pattern: Some("\\d{3}-\\d{3}-\\d{4}".into()),
            validation: Some(true),
            ..Explanation::new()
        },
    );

    // Recognizer 2: an NER model, slightly different span, higher confidence.
    let ner = Detection::new(
        RecognizerId::new("ner-model", "2024.1"),
        phone.clone(),
        TextLocation::new(10, 23),
        Confidence::new(0.95).unwrap(),
        Explanation {
            textual: Some("token classified as phone number".into()),
            ..Explanation::new()
        },
    );

    // Fuse both detections into one provenanced entity.
    let mut entity = fuse_max_confidence(vec![pattern, ner]);

    // The fusion kept the highest-confidence layer's location and score...
    assert_eq!(entity.label, phone);
    assert_eq!(entity.location, TextLocation::new(10, 23));
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

    // ...while retaining *both* original detections in the audit trail.
    assert_eq!(entity.provenance.detections.len(), 2);
    let merge = entity.provenance.merge.as_ref().expect("merge recorded");
    assert_eq!(merge.strategy, "max");
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
    use veil_core::primitive::geometry::{BoundingBox, Point, Polygon};

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
fn language_detection_records_provenance() {
    use veil_core::primitive::{LanguageDetection, LanguageProvenance};

    let en = LanguageTag::parse("en").unwrap();
    let detected = LanguageDetection::detected(en.clone(), Confidence::new(0.9));
    assert_eq!(detected.provenance, LanguageProvenance::Detected);
    assert_eq!(detected.confidence, Confidence::new(0.9));

    let asserted = LanguageDetection::asserted(en);
    assert_eq!(asserted.provenance, LanguageProvenance::Asserted);
    assert!(asserted.confidence.is_none());
}

#[test]
fn label_map_translates_raw_labels() {
    use veil_core::recognition::LabelMap;

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
fn recognizer_input_scopes_by_language_and_country() {
    use veil_core::recognition::RecognizerInput;

    let en_us = LanguageTag::parse("en-US").unwrap();
    let en = LanguageTag::parse("en").unwrap();
    let fr = LanguageTag::parse("fr").unwrap();

    // Primary-subtag matching: "en" matches "en-US".
    assert!(en.matches(&en_us));
    assert!(!en.matches(&fr));

    let input: RecognizerInput<Text> = RecognizerInput::new(TextData(String::new()))
        .with_language(en_us.clone())
        .with_country(CountryCode::from_alpha2("US").unwrap());

    // Empty scope always applies.
    assert!(input.applies_to_language(&[]));
    assert!(input.applies_to_country(&[]));
    // Matching scope applies; non-matching does not.
    assert!(input.applies_to_language(&[en]));
    assert!(!input.applies_to_language(&[fr]));
    assert!(input.applies_to_country(&[CountryCode::from_alpha2("US").unwrap()]));
    assert!(!input.applies_to_country(&[CountryCode::from_alpha2("GB").unwrap()]));
}

#[test]
fn anonymizer_trait_shape() {
    use veil_core::redaction::{Anonymizer, LeakProfile, OperatorId};

    /// A trivial `[LABEL]`-style replace operator, to exercise the
    /// trait shape and the pure `Replacement` model.
    struct Replace;

    impl Anonymizer<Text> for Replace {
        fn id(&self) -> OperatorId {
            OperatorId::new("replace", "1.0.0")
        }

        fn leak_profile(&self) -> LeakProfile {
            LeakProfile::Partial
        }

        async fn anonymize(
            &self,
            entity: &veil_core::entity::Entity<Text>,
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
