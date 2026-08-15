//! Integration tests for the shipped value-shaping operators end to end on the
//! public [`Anonymizer::anonymize`] path: generalizing a date, clamping an age
//! to a HIPAA ceiling, truncating a PAN, HMAC-tokenizing, localized bucket
//! labels, and custom fallbacks. Several are gated on the `datetime`/`hmac`
//! features and compile out when those are off.

mod fixtures;

use elide_core::entity::LabelRef;
use elide_core::modality::text::Text;
use elide_operator::operators::{Clamp, Keep};
use elide_redaction::{Anonymizer, Rule};
use fixtures::{anonymize_one, entity};

#[tokio::test]
async fn generalize_reduces_a_birthdate_to_the_year() {
    use elide_operator::operators::{DateGranularity, GeneralizeDate};

    //                    0         1
    //                    0123456789012345
    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("date_of_birth"),
            GeneralizeDate::new(DateGranularity::Year),
        )),
        "DOB: 1987-03-14",
        entity("date_of_birth", (5, 15)),
    )
    .await;
    assert_eq!(out, "DOB: 1987");
}

#[tokio::test]
async fn generalize_erases_an_unparseable_value_by_default() {
    use elide_operator::operators::{DateGranularity, GeneralizeDate};

    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("date_of_birth"),
            GeneralizeDate::new(DateGranularity::Year),
        )),
        "DOB: sometime",
        entity("date_of_birth", (5, 13)),
    )
    .await;
    assert_eq!(out, "DOB: ", "unparseable date erases");
}

#[tokio::test]
async fn clamp_caps_an_age_at_the_hipaa_ceiling() {
    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("age"),
            Clamp::new().with_ceiling(90.0, "90 or older"),
        )),
        "age 94",
        entity("age", (4, 6)),
    )
    .await;
    assert_eq!(out, "age 90 or older");
}

#[tokio::test]
async fn clamp_passes_an_in_range_age_through() {
    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("age"),
            Clamp::new().with_ceiling(90.0, "90 or older"),
        )),
        "age 73",
        entity("age", (4, 6)),
    )
    .await;
    assert_eq!(out, "age 73");
}

#[tokio::test]
async fn truncate_shortens_a_pan_to_bin_plus_last_four() {
    use elide_operator::operators::Truncate;

    //                    0               1
    //                    0123456789012345
    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("payment_card"),
            Truncate::new(6, 4),
        )),
        "4111111111111234",
        entity("payment_card", (0, 16)),
    )
    .await;
    // 16 chars in, 10 out: the middle six are physically gone, not masked.
    assert_eq!(out, "4111111234");
}

#[tokio::test]
async fn hmac_tokenizes_a_pan_deterministically() {
    use elide_operator::operators::HmacHash;

    let key = b"deployment-secret".to_vec();
    let a = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("payment_card"),
            HmacHash::sha256(key.clone()),
        )),
        "4111111111111234",
        entity("payment_card", (0, 16)),
    )
    .await;
    let b = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("payment_card"),
            HmacHash::sha256(key),
        )),
        "4111111111111234",
        entity("payment_card", (0, 16)),
    )
    .await;

    // Same key + value → same token (the property that makes it a token), and
    // the token replaces the PAN rather than echoing it.
    assert_eq!(a, b);
    assert_ne!(a, "4111111111111234", "the PAN must not survive");
    assert!(!a.is_empty(), "HMAC substitutes, never removes");
}

#[tokio::test]
async fn hmac_digest_changes_with_the_key() {
    use elide_operator::operators::HmacHash;

    let a = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("payment_card"),
            HmacHash::sha256(b"key-a".to_vec()),
        )),
        "4111111111111234",
        entity("payment_card", (0, 16)),
    )
    .await;
    let b = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("payment_card"),
            HmacHash::sha256(b"key-b".to_vec()),
        )),
        "4111111111111234",
        entity("payment_card", (0, 16)),
    )
    .await;
    assert_ne!(a, b, "a different key yields a different token");
}

#[tokio::test]
async fn clamp_renders_the_bucket_in_the_entity_language() {
    use elide_core::primitive::{LanguageTag, LocalizedText};

    // A French entity gets the French bucket label; the same policy would emit
    // the English one for an English entity.
    let bucket = LocalizedText::new("90 or older".to_owned())
        .with(LanguageTag::parse("fr").unwrap(), "90 ou plus".to_owned());

    // "âge 94": 'â' is two bytes, so "94" sits at bytes 5..7.
    let mut fr_entity = entity("age", (5, 7));
    fr_entity.language = Some(LanguageTag::parse("fr").unwrap());

    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("age"),
            Clamp::new().with_ceiling(90.0, bucket),
        )),
        "âge 94",
        fr_entity,
    )
    .await;
    assert_eq!(out, "âge 90 ou plus");
}

#[tokio::test]
async fn generalize_with_fallback_keeps_unparseable_intact() {
    use elide_operator::operators::{DateGranularity, GeneralizeDate, WithFallback};

    // A custom fallback (Keep) runs when GeneralizeDate declines the value,
    // instead of the bare operator's safe default (erase).
    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("date_of_birth"),
            WithFallback::new(GeneralizeDate::new(DateGranularity::Year), Keep),
        )),
        "DOB: n/a",
        entity("date_of_birth", (5, 8)),
    )
    .await;
    assert_eq!(out, "DOB: n/a");
}
