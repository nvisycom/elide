//! End-to-end shape test: two recognizers find the same entity, a
//! fusion combines them into one audited entity — exercising the
//! modality-generic model and the audit DAG the way the toolkit
//! fusion step would.

use elide_core::Result;
use elide_core::entity::audit::{AuditEvent, AuditKind, AuditLog, ModelEvent, PatternEvent};
use elide_core::entity::{Entity, EntityCoRef, Label, LabelCatalog, LabelLocale, LabelRef};
use elide_core::modality::Modality;
use elide_core::primitive::{Confidence, ConfidenceThreshold, CountryCode, Language, LanguageTag};

mod fixtures;
use fixtures::{Text, TextData, TextLocation, TextReplacement};

/// Build a single-recognition entity, the way a recognizer would.
fn recognized(
    label: &LabelRef,
    location: TextLocation,
    confidence: Confidence,
    event: AuditEvent<Text>,
) -> Entity<Text> {
    Entity::new(label.clone(), location, confidence, AuditLog::new(event))
}

/// A trivial "highest confidence wins" fusion: absorb every other entity's
/// trail, then record a deduplication event joining both trails' heads — what
/// the toolkit fusion step does, assembled here by hand from core parts.
fn fuse_max_confidence(mut entities: Vec<Entity<Text>>) -> Entity<Text> {
    entities.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    let mut base = entities.remove(0);
    let base_head = base.audit.head_hash();
    let mut parents = vec![base_head];
    for other in entities {
        parents.push(other.audit.head_hash());
        base.audit.absorb(other.audit.events().iter().cloned());
    }
    base.audit
        .record_fusion(AuditEvent::deduplication("max", base.confidence), parents);
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
        AuditEvent::pattern(
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
        AuditEvent::model(
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

    // Fuse both into one audited entity.
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
    assert_eq!(entity.audit.recognizers().count(), 2);
    assert_eq!(entity.audit.events().len(), 3);
    assert!(matches!(
        entity.audit.events().last().unwrap().kind,
        AuditKind::Deduplication(ref dedup) if dedup.strategy == "max"
    ));
    assert_eq!(
        entity.audit.final_confidence(),
        Some(Confidence::new(0.95).unwrap())
    );

    // The fusion event names both recognizers' heads as its parents, and the
    // whole DAG verifies.
    assert_eq!(entity.audit.events().last().unwrap().parents().len(), 2);
    assert!(entity.audit.verify().is_ok());
}

#[test]
fn label_catalog_resolves_refs() {
    let catalog: LabelCatalog = [
        Label::new("phone_number", "phone number").with_localization(
            LanguageTag::english(),
            LabelLocale::described("phone number", "A telephone number"),
        ),
        Label::new("email_address", "email address"),
    ]
    .into_iter()
    .collect();

    let en = LanguageTag::english();
    let phone = LabelRef::new("phone_number");
    assert_eq!(
        catalog.get(&phone).and_then(|l| l.description(&en)),
        Some("A telephone number")
    );
    assert!(catalog.contains(&LabelRef::new("email_address")));
    assert!(!catalog.contains(&LabelRef::new("ssn")));

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
    use elide_core::primitive::{BoundingBox, Point, Polygon};

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
    use elide_core::recognition::{RecognizerContext, Scope};

    let en_us = LanguageTag::parse("en-US").unwrap();
    let en = LanguageTag::parse("en").unwrap();
    let fr = LanguageTag::parse("fr").unwrap();

    // Primary-subtag matching: "en" matches "en-US".
    assert!(en.matches(&en_us));
    assert!(!en.matches(&fr));

    // Assertions live on the scope; the query methods live on the
    // context, which borrows the scope.
    let scope = Scope::new()
        .with_language(Language::asserted(en_us.clone()))
        .with_country(CountryCode::from_alpha2("US").unwrap());
    let ctx: RecognizerContext<'_, Text> = RecognizerContext::new(&scope);

    // The asserted language is the primary one.
    assert_eq!(ctx.primary_language(), Some(&en_us));

    // Country scope: empty always applies; matching applies, non-matching not.
    assert!(ctx.applies_to_country(&[]));
    assert!(ctx.applies_to_country(&[CountryCode::from_alpha2("US").unwrap()]));
    assert!(!ctx.applies_to_country(&[CountryCode::from_alpha2("GB").unwrap()]));

    // Language scope: a matching or empty scope applies, a mismatch does not.
    // The only language is the asserted en-US, so the asserted-OR-detected
    // `applies_to_language` and the asserted-only `applies_to_asserted_language`
    // agree here — each rule scope is checked through both.
    let en_scope = [en];
    let fr_scope = [fr];
    assert!(ctx.applies_to_language(&[]));
    assert!(ctx.applies_to_language(&en_scope));
    assert!(!ctx.applies_to_language(&fr_scope));
    assert!(ctx.applies_to_asserted_language(&[]));
    assert!(ctx.applies_to_asserted_language(&en_scope));
    assert!(!ctx.applies_to_asserted_language(&fr_scope));
}

#[test]
fn a_detected_language_never_filters() {
    use elide_core::recognition::{RecognizerContext, Scope};

    let de = LanguageTag::parse("de").unwrap();
    let es = LanguageTag::parse("es").unwrap();

    // No asserted language; a detector reports Spanish with high confidence.
    let scope = Scope::new();
    let mut ctx: RecognizerContext<'_, Text> = RecognizerContext::new(&scope);
    ctx.detect_language(Language::detected(es).with_confidence(Confidence::clamped(0.9)));

    // A German-scoped rule still runs: detection is unreliable and must never
    // suppress a match — only a caller assertion filters by language.
    let de_scope = [de];
    assert!(ctx.applies_to_asserted_language(&de_scope));
    // The old asserted-OR-detected filter WOULD suppress it (detected es ≠ de).
    assert!(!ctx.applies_to_language(&de_scope));

    // `asserted_languages` excludes the detected one — the caller asserted
    // nothing — so per-language context selection stays permissive rather than
    // keying on the (unreliable) detected `es`.
    assert!(ctx.asserted_languages().is_empty());
    assert_eq!(
        ctx.ranked_languages().len(),
        1,
        "detection is still recorded"
    );
}

#[test]
fn recognizer_context_carries_annotations() {
    use elide_core::entity::LabelRef;
    use elide_core::modality::text::TextLocation;
    use elide_core::primitive::Confidence;
    use elide_core::recognition::annotation::{Annotations, Exclusion, Inclusion};
    use elide_core::recognition::{RecognizerContext, Scope};

    let inclusion = Inclusion::new(TextLocation::new(0, 5))
        .with_name("uploaded selection")
        .with_label(LabelRef::new("PERSON"))
        .with_confidence(Confidence::new(0.9).unwrap());
    let exclusion = Exclusion::new(TextLocation::new(10, 20));
    let scope = Scope::new();
    let annotations: Annotations<Text> = Annotations::new()
        .with_inclusions(vec![inclusion])
        .with_exclusions(vec![exclusion]);
    let ctx = RecognizerContext::new(&scope).with_annotations(&annotations);

    assert_eq!(ctx.inclusions().len(), 1);
    assert_eq!(ctx.inclusions()[0].location, TextLocation::new(0, 5));
    assert_eq!(
        ctx.inclusions()[0].name.as_deref(),
        Some("uploaded selection")
    );
    assert_eq!(ctx.inclusions()[0].label, Some(LabelRef::new("PERSON")));
    assert_eq!(
        ctx.inclusions()[0].confidence,
        Some(Confidence::new(0.9).unwrap())
    );

    assert_eq!(ctx.exclusions().len(), 1);
    assert_eq!(ctx.exclusions()[0].location, TextLocation::new(10, 20));
}

#[test]
fn operator_trait_shape() {
    use elide_core::operator::{LeakProfile, Operator, OperatorId};

    /// A trivial `[LABEL]`-style replace operator, to exercise the
    /// trait shape and the pure `Replacement` model.
    struct Replace;

    #[async_trait::async_trait]
    impl Operator<Text> for Replace {
        fn id(&self) -> OperatorId {
            OperatorId::new("replace", "1.0.0")
        }

        fn leak_profile(&self) -> LeakProfile {
            LeakProfile::Partial
        }

        async fn anonymize(
            &self,
            entity: &Entity<Text>,
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
