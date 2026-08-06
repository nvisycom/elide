//! End-to-end fixtures for the value-shaping operators added for the
//! regulatory-compliance operator set: `GeneralizeDate` (HIPAA date
//! generalization), `Clamp` (HIPAA age ceiling), `Truncate` (PCI PAN
//! truncation), and `HmacHash` (PCI keyed hash). Each runs through the
//! real `Anonymizer::plan` path so the label→operator resolution and the
//! `DataReader` slice are exercised, not just the operator in isolation.

use elide::redaction::Anonymizer;
use elide::redaction::operators::{
    Clamp, DateGranularity, GeneralizeDate, HmacHash, Keep, Truncate, WithFallback,
};
use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
use elide_core::entity::{Entity, LabelRef};
use elide_core::primitive::{Confidence, LanguageTag, LocalizedText};

mod fixtures;
use fixtures::{Text, TextLocation, TextReplacement, TextSource};

fn entity(label: &str, loc: (usize, usize)) -> Entity<Text> {
    let location = TextLocation::new(loc.0, loc.1);
    let event = Event::pattern("test", Confidence::MAX, location.clone(), PatternEvent::default());
    Entity::new(
        LabelRef::new(label.to_owned()),
        location,
        Confidence::MAX,
        Provenance::new(event),
    )
}

async fn plan_one(anonymizer: Anonymizer<Text>, source: &TextSource, e: Entity<Text>) -> TextReplacement {
    let mut entities = vec![e];
    let items = anonymizer
        .plan(&mut entities, source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(items.len(), 1, "exactly one redaction planned");
    items.into_iter().next().unwrap().1
}

#[tokio::test]
async fn generalize_reduces_a_birthdate_to_the_year() {
    //                                  0         1
    //                                  0123456789012345
    let source = TextSource::new("DOB: 1987-03-14");
    let anonymizer = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("date_of_birth"), GeneralizeDate::new(DateGranularity::Year));

    let out = plan_one(anonymizer, &source, entity("date_of_birth", (5, 15))).await;
    assert_eq!(out, TextReplacement::substituted("1987"));
}

#[tokio::test]
async fn generalize_erases_an_unparseable_value_by_default() {
    let source = TextSource::new("DOB: sometime");
    let anonymizer = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("date_of_birth"), GeneralizeDate::new(DateGranularity::Year));

    let out = plan_one(anonymizer, &source, entity("date_of_birth", (5, 13))).await;
    assert_eq!(out, TextReplacement::Removed, "unparseable date erases");
}

#[tokio::test]
async fn clamp_caps_an_age_at_the_hipaa_ceiling() {
    let source = TextSource::new("age 94");
    let anonymizer = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("age"), Clamp::new().with_ceiling(90.0, "90 or older"));

    let out = plan_one(anonymizer, &source, entity("age", (4, 6))).await;
    assert_eq!(out, TextReplacement::substituted("90 or older"));
}

#[tokio::test]
async fn clamp_passes_an_in_range_age_through() {
    let source = TextSource::new("age 73");
    let anonymizer = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("age"), Clamp::new().with_ceiling(90.0, "90 or older"));

    let out = plan_one(anonymizer, &source, entity("age", (4, 6))).await;
    assert_eq!(out, TextReplacement::substituted("73"));
}

#[tokio::test]
async fn truncate_shortens_a_pan_to_bin_plus_last_four() {
    //                            0               1
    //                            0123456789012345
    let source = TextSource::new("4111111111111234");
    let anonymizer = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("payment_card"), Truncate::new(6, 4));

    let out = plan_one(anonymizer, &source, entity("payment_card", (0, 16))).await;
    // 16 chars in, 10 out: the middle six are physically gone, not masked.
    assert_eq!(out, TextReplacement::substituted("4111111234"));
}

#[tokio::test]
async fn hmac_tokenizes_a_pan_deterministically() {
    let source = TextSource::new("4111111111111234");
    let key = b"deployment-secret".to_vec();

    let a = plan_one(
        Anonymizer::<Text>::new().with_label(LabelRef::new("payment_card"), HmacHash::sha256(key.clone())),
        &source,
        entity("payment_card", (0, 16)),
    )
    .await;
    let b = plan_one(
        Anonymizer::<Text>::new().with_label(LabelRef::new("payment_card"), HmacHash::sha256(key)),
        &source,
        entity("payment_card", (0, 16)),
    )
    .await;

    // Same key + value → same token (the property that makes it a token),
    // and the token replaces the PAN rather than echoing it.
    assert_eq!(a, b);
    match a {
        TextReplacement::Substituted(digest) => {
            assert_ne!(digest, "4111111111111234", "the PAN must not survive");
        }
        TextReplacement::Removed => panic!("HMAC substitutes, never removes"),
    }
}

#[tokio::test]
async fn hmac_digest_changes_with_the_key() {
    let source = TextSource::new("4111111111111234");
    let a = plan_one(
        Anonymizer::<Text>::new().with_label(LabelRef::new("payment_card"), HmacHash::sha256(b"key-a".to_vec())),
        &source,
        entity("payment_card", (0, 16)),
    )
    .await;
    let b = plan_one(
        Anonymizer::<Text>::new().with_label(LabelRef::new("payment_card"), HmacHash::sha256(b"key-b".to_vec())),
        &source,
        entity("payment_card", (0, 16)),
    )
    .await;
    assert_ne!(a, b, "a different key yields a different token");
}

#[tokio::test]
async fn clamp_renders_the_bucket_in_the_entity_language() {
    // A French entity gets the French bucket label; the same policy would
    // emit the English one for an English entity.
    let source = TextSource::new("âge 94");
    let bucket = LocalizedText::new("90 or older".to_owned())
        .with(LanguageTag::parse("fr").unwrap(), "90 ou plus".to_owned());
    let anonymizer = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("age"), Clamp::new().with_ceiling(90.0, bucket));

    let mut fr_entity = entity("age", (5, 7));
    fr_entity.language = Some(LanguageTag::parse("fr").unwrap());
    let out = plan_one(anonymizer, &source, fr_entity).await;
    assert_eq!(out, TextReplacement::substituted("90 ou plus"));
}

#[tokio::test]
async fn generalize_with_fallback_keeps_unparseable_intact() {
    // A custom fallback (Keep) runs when GeneralizeDate declines the value,
    // instead of the bare operator's safe default (erase).
    let source = TextSource::new("DOB: n/a");
    let anonymizer = Anonymizer::<Text>::new().with_label(
        LabelRef::new("date_of_birth"),
        WithFallback::new(GeneralizeDate::new(DateGranularity::Year), Keep),
    );

    let out = plan_one(anonymizer, &source, entity("date_of_birth", (5, 8))).await;
    assert_eq!(out, TextReplacement::substituted("n/a"));
}
