//! End-to-end analyzer test: two recognizers find overlapping
//! `PHONE_NUMBER`s; the analyzer fuses them, and a `FilterLayer` drops a
//! low-confidence stray.

use elide::detection::Analyzer;
use elide::detection::calibrate::{CalibrateLayer, CalibrationMap};
use elide::detection::filter::FilterLayer;
use elide::detection::reconcile::{Merging, ReconcileLayer, Structural};
use elide_core::Result;
use elide_core::entity::audit::{AuditEvent, AuditKind, AuditLog, PatternEvent};
use elide_core::entity::{Entity, LabelRef};
use elide_core::primitive::{Confidence, ConfidenceThreshold};
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, RecognizerId, Scope};

use crate::support::{SourceRef, Text, TextData, TextLocation};

/// Builds an entity carrying one recognition event, the way a recognizer
/// would.
fn detected(recognizer: &str, label: &str, loc: (usize, usize), conf: f32) -> Entity<Text> {
    let label = LabelRef::new(label.to_owned());
    let location = TextLocation::new(loc.0, loc.1);
    let confidence = Confidence::new(conf).unwrap();
    let event = AuditEvent::pattern(
        recognizer.to_owned(),
        confidence,
        location.clone(),
        PatternEvent {
            name: label.as_str().into(),
            ..PatternEvent::default()
        },
    );
    Entity::new(label, location, confidence, AuditLog::new(event))
}

/// Like [`detected`], but the location also carries a raw source reference — the
/// pointer a source-mapping codec (markup, DOCX) attaches on lift.
fn detected_with_source(
    recognizer: &str,
    label: &str,
    loc: (usize, usize),
    conf: f32,
    source: SourceRef,
) -> Entity<Text> {
    let mut entity = detected(recognizer, label, loc, conf);
    entity.location = entity.location.with_source([source]);
    entity
}

/// A recognizer that just replays a fixed entity list.
struct Fixed(Vec<Entity<Text>>);

#[async_trait::async_trait]
impl Recognizer<Text> for Fixed {
    fn id(&self) -> RecognizerId {
        RecognizerId::new("fixed", "1.0.0")
    }

    async fn recognize(
        &self,
        _data: &TextData,
        _ctx: &RecognizerContext<'_, Text>,
    ) -> Result<Recognition<Text>> {
        Ok(self.0.clone().into())
    }
}

#[tokio::test]
async fn analyze_fuses_resolves_filters() {
    // Recognizer A: a phone at 10..22 and a weak stray at 40..44.
    let a = Fixed(vec![
        detected("pattern", "PHONE_NUMBER", (10, 22), 0.8),
        detected("pattern", "WEAK", (40, 44), 0.1),
    ]);
    // Recognizer B: the same phone, slightly wider, higher confidence.
    let b = Fixed(vec![detected("ner", "PHONE_NUMBER", (10, 23), 0.95)]);

    let analyzer = Analyzer::<Text>::new()
        .with_recognizer(a)
        .with_recognizer(b)
        .with_layer(CalibrateLayer::new(CalibrationMap::new())) // identity (empty)
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));

    let mut entities = analyzer
        .analyze(TextData::new(""), &Scope::new())
        .await
        .unwrap()
        .entities;

    // The two PHONE_NUMBER detections fused into one; the weak stray was
    // filtered out below the 0.35 baseline.
    assert_eq!(entities.len(), 1);
    let phone = entities.pop().unwrap();
    assert_eq!(phone.label, LabelRef::new("PHONE_NUMBER"));
    // Fusion kept the higher-confidence, larger span and recorded both
    // recognitions plus a deduplication event.
    assert_eq!(phone.confidence, Confidence::new(0.95).unwrap());
    assert_eq!(phone.location, TextLocation::new(10, 23));
    assert_eq!(phone.audit.recognizers().count(), 2);
    // The trail: 2 recognition events + 1 deduplication event.
    assert_eq!(phone.audit.events().len(), 3);
    let last = phone.audit.events().last().unwrap();
    assert!(matches!(
        last.kind,
        AuditKind::Deduplication(ref d) if d.strategy == "max"
    ));
    assert_eq!(phone.audit.final_confidence(), Some(phone.confidence));
}

#[cfg(feature = "usage")]
#[tokio::test]
async fn analyze_records_per_recognizer_usage() {
    // Two recognizers: A finds 2, B finds 1. Each should get one Usage entry
    // carrying its id, its own found-count (measured before reduction), a
    // duration, and — being pure-CPU doubles — no model detail.
    let a = Fixed(vec![
        detected("pattern", "PHONE_NUMBER", (10, 22), 0.8),
        detected("pattern", "WEAK", (40, 44), 0.1),
    ]);
    let b = Fixed(vec![detected("ner", "PHONE_NUMBER", (10, 23), 0.95)]);

    let analyzer = Analyzer::<Text>::new()
        .with_recognizer(a)
        .with_recognizer(b)
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));

    let analysis = analyzer
        .analyze(TextData::new(""), &Scope::new())
        .await
        .unwrap();

    // One usage entry per recognizer, in registration order.
    assert_eq!(analysis.usage.len(), 2);
    for usage in &analysis.usage {
        assert_eq!(usage.id.name, "fixed");
        assert!(usage.model.is_none(), "a pure-CPU double reports no model");
    }
    // Counts reflect what each recognizer returned (pre-reduction): 2 and 1.
    assert_eq!(analysis.usage[0].count, Some(2));
    assert_eq!(analysis.usage[1].count, Some(1));
}

#[tokio::test]
async fn fusion_keeps_both_operands_source_refs() {
    // Two overlapping detections of the same label, each carrying a distinct raw
    // source reference (as a markup/DOCX codec would attach). Reconciliation
    // fuses them; the surviving entity must keep *both* source refs, normalized
    // — so a client can still point at every source run behind the fused span.
    let a = Fixed(vec![detected_with_source(
        "pattern",
        "PHONE_NUMBER",
        (10, 22),
        0.8,
        SourceRef::in_part(200..212, "word/document.xml"),
    )]);
    let b = Fixed(vec![detected_with_source(
        "ner",
        "PHONE_NUMBER",
        (10, 23),
        0.95,
        SourceRef::in_part(300..313, "word/header1.xml"),
    )]);

    let analyzer = Analyzer::<Text>::new()
        .with_recognizer(a)
        .with_recognizer(b)
        .with_layer(ReconcileLayer::same_label(Merging::max()));

    let mut entities = analyzer
        .analyze(TextData::new(""), &Scope::new())
        .await
        .unwrap()
        .entities;

    assert_eq!(entities.len(), 1);
    let phone = entities.pop().unwrap();
    // Both source refs survive the fusion, in canonical (part-then-range) order.
    assert_eq!(
        phone.location.source,
        vec![
            SourceRef::in_part(200..212, "word/document.xml"),
            SourceRef::in_part(300..313, "word/header1.xml"),
        ]
    );
}

#[tokio::test]
async fn analyze_stamps_language_from_recognized_range() {
    use elide_core::primitive::{Language, LanguageTag};

    // An entity carrying a recognized_range (where it was found in the text).
    let mut e = detected("pattern", "PERSON", (0, 5), 0.9);
    e.recognized_range = Some(0..5);

    let analyzer = Analyzer::<Text>::new().with_recognizer(Fixed(vec![e]));

    // The caller asserts the document language; it applies span-less (whole
    // payload), so every ranged entity is attributed to it.
    let de = Language::asserted(LanguageTag::parse("de").unwrap());
    let scope = Scope::new().with_language(de);

    let entities = analyzer
        .analyze(TextData::new("hello"), &scope)
        .await
        .unwrap()
        .entities;
    assert_eq!(entities.len(), 1);
    assert_eq!(
        entities[0].language.as_ref().map(|l| l.primary_language()),
        Some("de")
    );
}

#[test]
fn calibrate_scales_by_originating_recognizer() {
    use elide::detection::Layer;

    // "pattern" always fires at 1.0; calibrate it down by 0.5.
    let calibration: CalibrationMap = [("pattern", 0.5), ("ner", 0.8)].into_iter().collect();
    let layer = CalibrateLayer::new(calibration);

    let entities = vec![
        detected("pattern", "PHONE_NUMBER", (0, 4), 1.0),
        detected("ner", "PERSON", (5, 9), 1.0),
        detected("unknown", "EMAIL_ADDRESS", (10, 14), 0.9),
    ];
    let out = layer.apply(entities);
    assert!(out.dropped.is_empty(), "calibrate never drops");

    // pattern 1.0 * 0.5 = 0.5; ner 1.0 * 0.8 = 0.8; unknown unchanged.
    assert_eq!(out.kept[0].confidence, Confidence::new(0.5).unwrap());
    assert_eq!(out.kept[1].confidence, Confidence::new(0.8).unwrap());
    assert_eq!(out.kept[2].confidence, Confidence::new(0.9).unwrap());
}

/// A strong out-of-catalog detection reconciles first, subsuming a weak
/// in-catalog match nested inside it, and is only then culled from the output
/// by the request catalog. This is the `IBAN`-covers-`GB29` case: the caller's
/// catalog excludes `IBAN` but the recognizer still emits it, so reconciliation
/// can use it to drop the loose `DRIVERS_LICENSE` prefix before the catalog
/// restriction removes the `IBAN` itself.
#[tokio::test]
async fn out_of_catalog_container_subsumes_then_is_culled() {
    use elide_core::entity::{Label, LabelCatalog};

    // A validated IBAN (0.85) fully containing a loose driver's-license
    // prefix (0.4), plus an in-catalog SSN elsewhere.
    let recognizer = Fixed(vec![
        detected("pattern", "IBAN", (0, 27), 0.85),
        detected("pattern", "DRIVERS_LICENSE", (0, 4), 0.4),
        detected("pattern", "GOVERNMENT_ID", (40, 51), 0.85),
    ]);

    // Catalog declares the two in-catalog labels, not IBAN.
    let catalog: LabelCatalog = [
        Label::new("DRIVERS_LICENSE", "driver's license"),
        Label::new("GOVERNMENT_ID", "government id"),
    ]
    .into_iter()
    .collect();

    let analyzer = Analyzer::<Text>::new()
        .with_recognizer(recognizer)
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));

    let scope = Scope::new().with_catalog(catalog);
    let entities = analyzer
        .analyze(TextData::new(""), &scope)
        .await
        .unwrap()
        .entities;

    let labels: Vec<&str> = entities.iter().map(|e| e.label.as_str()).collect();
    // The nested driver's-license prefix was subsumed by the IBAN.
    assert!(
        !labels.contains(&"DRIVERS_LICENSE"),
        "GB29 must not survive"
    );
    // The IBAN did its job then dropped out (not in catalog).
    assert!(
        !labels.contains(&"IBAN"),
        "out-of-catalog IBAN must not be output"
    );
    // The in-catalog SSN survives.
    assert_eq!(labels, ["GOVERNMENT_ID"]);
}
