//! Context-enhanced pattern recognition over image and audio.
//!
//! The pattern recognizer reads its text from the OCR `Layout` / STT
//! `Transcription` artifact, finds a match, and the `Enhanced` adapter boosts
//! its confidence from a context keyword in that same text — all before the
//! match is lifted to a native location. Proves enhancement works for image
//! and audio (whose location is a region / time span, not a byte range) and
//! that the resulting entity carries the native location, not a stream offset.
#![cfg(all(feature = "ocr", feature = "stt"))]

use elide::entity::LabelRef;
use elide::modality::audio::{Audio, AudioData, TranscriptSegment, TranscriptWord, Transcription};
use elide::modality::image::{Image, ImageData, ImageLocation, Layout, LayoutBlock, LayoutWord};
use elide::primitive::{BoundingBox, Confidence, ConfidenceThreshold, Dimensions, Point, TimeSpan};
use elide::recognition::pattern::{PatternRecognizer, Regex, Variant};
use elide::recognition::{Recognizer, RecognizerContext, Scope};

/// A pattern recognizer that matches a 9-digit run, boosted by the keyword
/// "ssn" nearby, wrapped in the `Enhanced` context layer.
fn ssn_recognizer() -> impl Recognizer<Image> + Recognizer<Audio> {
    let variant = Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
        .expect("variant builds")
        .with_score(Confidence::clamped(0.5));
    let regex = Regex::builder()
        .with_name("ssn")
        .with_label(LabelRef::new("US_SSN"))
        .with_context(vec!["ssn".to_owned()])
        .with_variants(vec![variant])
        .build()
        .expect("rule builds");
    PatternRecognizer::builder()
        .with_pattern(regex)
        .build_context_enhanced()
        .expect("recognizer builds")
}

fn img_loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
    ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
}

#[tokio::test]
async fn image_context_boosts_and_keeps_the_native_region() {
    // OCR text: "ssn 123-45-6789", with per-word boxes.
    let block = LayoutBlock::new(img_loc(0.0, 0.0, 200.0, 20.0), "ssn 123-45-6789").with_words(vec![
        LayoutWord::new(img_loc(0.0, 0.0, 40.0, 20.0), "ssn"),
        LayoutWord::new(img_loc(45.0, 0.0, 155.0, 20.0), "123-45-6789"),
    ]);
    let scope = Scope::<Image>::new();
    let mut ctx = RecognizerContext::new(&scope);
    ctx.artifacts.insert(Layout::new(vec![block]));

    let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(200, 20));
    let entities = ssn_recognizer().recognize(&data, &ctx).await.unwrap();

    assert_eq!(entities.len(), 1);
    let entity = &entities[0];
    // The "ssn" keyword boosted 0.5 -> above baseline.
    assert!(entity.confidence > Confidence::new(0.5).unwrap());
    assert!(ConfidenceThreshold::BASELINE.passes(entity.confidence));
    // The entity addresses the image region of the match, not a byte range.
    assert_eq!(entity.location.bounding_box.min.x, 45.0);
    assert!(entity.location.bounding_box.area() > 0.0);
}

#[tokio::test]
async fn audio_context_boosts_and_keeps_the_native_timespan() {
    // Transcript "ssn 123-45-6789" with per-word timings.
    let segment = TranscriptSegment::new(TimeSpan::from_millis(0, 1500), "ssn 123-45-6789")
        .with_words(vec![
            TranscriptWord::new(TimeSpan::from_millis(0, 300), "ssn"),
            TranscriptWord::new(TimeSpan::from_millis(400, 1500), "123-45-6789"),
        ]);
    let scope = Scope::<Audio>::new();
    let mut ctx = RecognizerContext::new(&scope);
    ctx.artifacts.insert(Transcription::new(vec![segment]));

    let data = AudioData::new(bytes::Bytes::new());
    let entities = ssn_recognizer().recognize(&data, &ctx).await.unwrap();

    assert_eq!(entities.len(), 1);
    let entity = &entities[0];
    assert!(entity.confidence > Confidence::new(0.5).unwrap());
    // The entity addresses the audio time span of the match.
    assert_eq!(entity.location.span.start_millis(), 400);
    assert!(!entity.location.span.is_empty());
}
