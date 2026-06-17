//! End-to-end analyzer test: two recognizers find overlapping
//! `PHONE_NUMBER`s; the analyzer fuses them, and a `FilterLayer` drops a
//! low-confidence stray.

use veil_core::Error;
use veil_core::entity::{Entity, LabelRef};
use veil_core::primitive::Confidence;
use veil_core::primitive::ConfidenceThreshold;
use veil_core::provenance::{Event, EventKind, PatternEvent, Provenance};
use veil_core::recognition::{Recognizer, RecognizerId, RecognizerInput, RecognizerOutput};
use veil_toolkit::Analyzer;
use veil_toolkit::deduplication::calibrate::{CalibrateLayer, CalibrationMap};
use veil_toolkit::deduplication::filter::FilterLayer;
use veil_toolkit::deduplication::fuse::{FuseLayer, MaxConfidence};
use veil_toolkit::deduplication::resolve::{HighestConfidence, ResolveLayer};

mod fixtures;
use fixtures::{Text, TextData, TextLocation};

/// Builds an entity carrying one recognition event, the way a recognizer
/// would.
fn detected(recognizer: &str, label: &str, loc: (usize, usize), conf: f32) -> Entity<Text> {
    let label = LabelRef::new(label.to_owned());
    let location = TextLocation::new(loc.0, loc.1);
    let confidence = Confidence::new(conf).unwrap();
    let event = Event::pattern(
        recognizer.to_owned(),
        confidence,
        location.clone(),
        PatternEvent {
            name: label.as_str().into(),
            ..PatternEvent::default()
        },
    );
    Entity::new(label, location, confidence, Provenance::new(event))
}

/// A recognizer that just replays a fixed entity list.
struct Fixed(Vec<Entity<Text>>);

impl Recognizer<Text> for Fixed {
    fn id(&self) -> RecognizerId {
        RecognizerId::new("fixed", "1.0.0")
    }

    async fn recognize(
        &self,
        _input: &RecognizerInput<Text>,
    ) -> Result<RecognizerOutput<Text>, Error> {
        Ok(RecognizerOutput::new(self.0.clone()))
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
        .with_layer(FuseLayer::new(MaxConfidence))
        .with_layer(ResolveLayer::new(HighestConfidence))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));

    let mut entities = analyzer
        .analyze(RecognizerInput::new(TextData::new("")))
        .await
        .unwrap();

    // The two PHONE_NUMBER detections fused into one; the weak stray was
    // filtered out below the 0.35 baseline.
    assert_eq!(entities.len(), 1);
    let phone = entities.pop().unwrap();
    assert_eq!(phone.label, LabelRef::new("PHONE_NUMBER"));
    // Fusion kept the higher-confidence, larger span and recorded both
    // recognitions plus a deduplication event.
    assert_eq!(phone.confidence, Confidence::new(0.95).unwrap());
    assert_eq!(phone.location, TextLocation::new(10, 23));
    assert_eq!(phone.provenance.recognizers().count(), 2);
    // The trail: 2 recognition events + 1 deduplication event.
    assert_eq!(phone.provenance.events.len(), 3);
    let last = phone.provenance.events.last().unwrap();
    assert!(matches!(
        last.kind,
        EventKind::Deduplication { ref strategy } if strategy == "max"
    ));
    assert_eq!(phone.provenance.final_confidence(), Some(phone.confidence));
}

#[test]
fn calibrate_scales_by_originating_recognizer() {
    use veil_toolkit::deduplication::Layer;

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
