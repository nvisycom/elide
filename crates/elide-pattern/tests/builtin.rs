//! End-to-end: shipped patterns + dictionaries against the
//! cross-jurisdiction (`world`) fixtures.
//!
//! Each test scans one `testdata/inputs/<domain>.txt` fixture
//! through a recognizer wired with every shipped pattern and
//! dictionary, then asserts the entities a real document of that
//! domain is expected to surface (substring + label, not
//! byte-offset, so fixtures and regexes can evolve without
//! brittle churn).

mod fixtures;

use elide_core::entity::builtins;
use elide_core::{assert_label_absent, assert_label_present};
use fixtures::{scan, scan_in_country};

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/contact.txt")).await;
    assert_match!(
        &text,
        &entities,
        builtins::EMAIL_ADDRESS.to_ref(),
        "alice.johnson@example.com",
    );
    assert_match!(
        &text,
        &entities,
        builtins::URL.to_ref(),
        "https://docs.example.com/proposal",
    );
    assert_match!(
        &text,
        &entities,
        builtins::URL.to_ref(),
        "http://backup.example.org/proposal-v2",
    );
    assert_label_present!(entities, builtins::PHONE_NUMBER.to_ref());
}

#[tokio::test]
async fn builtin_credentials() {
    let (_, entities) = scan(include_str!("../testdata/inputs/credentials.txt")).await;
    assert_label_present!(entities, builtins::API_KEY.to_ref());
    assert_label_present!(entities, builtins::PRIVATE_KEY.to_ref());
    assert_label_present!(entities, builtins::AUTH_TOKEN.to_ref());
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/finance.txt")).await;
    assert_match!(
        &text,
        &entities,
        builtins::IBAN.to_ref(),
        "GB29 NWBK 6016 1331 9268 19",
    );
    assert_match!(&text, &entities, builtins::SWIFT_CODE.to_ref(), "NWBKGB2L");
    assert_match!(
        &text,
        &entities,
        builtins::PAYMENT_CARD.to_ref(),
        "4539 1488 0343 6467",
    );
    assert_match!(
        &text,
        &entities,
        builtins::CRYPTO_ADDRESS.to_ref(),
        "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
    );
    assert_match!(
        &text,
        &entities,
        builtins::CRYPTO_ADDRESS.to_ref(),
        "0x742d35Cc6634C0532925a3b844Bc9e7595f6E842",
    );
    // Currency dictionaries pick up `USD`, `EUR`, `Tether`, `USDC`.
    assert_label_present!(entities, builtins::CURRENCY.to_ref());
}

#[tokio::test]
async fn builtin_network() {
    let (text, entities) = scan(include_str!("../testdata/inputs/network.txt")).await;
    assert_match!(
        &text,
        &entities,
        builtins::IP_ADDRESS.to_ref(),
        "192.168.1.42",
    );
    assert_match!(&text, &entities, builtins::IP_ADDRESS.to_ref(), "10.0.0.7");
    assert_match!(
        &text,
        &entities,
        builtins::IP_ADDRESS.to_ref(),
        "203.0.113.55",
    );
    assert_match!(
        &text,
        &entities,
        builtins::IP_ADDRESS.to_ref(),
        "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
    );
    assert_match!(
        &text,
        &entities,
        builtins::MAC_ADDRESS.to_ref(),
        "00:1A:2B:3C:4D:5E",
    );
    assert_match!(
        &text,
        &entities,
        builtins::MAC_ADDRESS.to_ref(),
        "3C-22-FB-A1-B2-C3",
    );
}

#[tokio::test]
async fn builtin_personal() {
    let (text, entities) = scan(include_str!("../testdata/inputs/personal.txt")).await;
    assert_match!(
        &text,
        &entities,
        builtins::DATE_OF_BIRTH.to_ref(),
        "04/22/1979",
    );
    assert_match!(
        &text,
        &entities,
        builtins::DATE_TIME.to_ref(),
        "2024-06-15T09:30:00Z",
    );
    assert_label_present!(entities, builtins::NATIONALITY.to_ref());
    assert_label_present!(entities, builtins::LANGUAGE.to_ref());
}

/// A phone-shaped digit run that is actually part of a longer token must not
/// surface as a phone number. The bare digit-led form of the phone regex
/// carries a `\b`, so it has to start a token; the `+country` and `(area)`
/// forms begin with a non-word character and so stay matchable even when glued
/// to a preceding token. The `phone` validator (libphonenumber) rejects the
/// rest. The country is asserted because that is the adversarial case — it
/// enables national-format acceptance.
#[tokio::test]
async fn phone_does_not_match_inside_longer_tokens() {
    // Mid-token start: `212 555 0100` is a valid US number shape, but here it
    // begins immediately after letters, so it is part of `X212…`, not a phone.
    // The digit-led `\b` is what rejects this (the validator would accept it).
    let (_, entities) = scan_in_country("ref X212 555 0100 end", "US").await;
    assert_label_absent!(entities, builtins::PHONE_NUMBER.to_ref());
    let (_, entities) = scan_in_country("ABC212 555 0100", "US").await;
    assert_label_absent!(entities, builtins::PHONE_NUMBER.to_ref());

    // Spaced IBAN: a run like `61 1090 1014 0000 0712` can be matched by the
    // phone regex, but is not a valid phone number.
    let (_, entities) = scan_in_country("PL61 1090 1014 0000 0712 1981 2874", "PL").await;
    assert_label_absent!(entities, builtins::PHONE_NUMBER.to_ref());

    // Payment card number.
    let (_, entities) = scan_in_country("4111 1111 1111 1111", "US").await;
    assert_label_absent!(entities, builtins::PHONE_NUMBER.to_ref());

    // Prefixed IBAN.
    let (_, entities) = scan_in_country("GB29 NWBK 6016 1331 9268 19", "GB").await;
    assert_label_absent!(entities, builtins::PHONE_NUMBER.to_ref());

    // A genuine number with surrounding words is still detected.
    let (_, entities) = scan_in_country("call +1 212 555 0100 today", "US").await;
    assert_label_present!(entities, builtins::PHONE_NUMBER.to_ref());

    // A `+country` number glued to a preceding token (no separator) still
    // matches — the boundary requirement is only on the bare digit-led form.
    let (_, entities) = scan_in_country("a@b.com+1 (628) 555-0175", "US").await;
    assert_label_present!(entities, builtins::PHONE_NUMBER.to_ref());
}
