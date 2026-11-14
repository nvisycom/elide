//! [`FormatRegistry`] end to end through the built-in formats: resolve
//! content to a format, decode it, redact it, and re-encode it.

#![cfg(feature = "txt")]

use elide_codec::content::ContentData;
use elide_core::ErrorKind;
use elide_core::modality::DataWriter;
use elide_core::modality::text::{Text, TextLocation, TextReplacement};
use elide_core::redaction::{HiddenContentPolicy, Redactions};
use elide_format::FormatRegistry;

#[tokio::test]
async fn decode_resolves_by_extension() {
    let reg = FormatRegistry::with_builtin();
    let mut document = reg
        .decode("hello\nworld\n", "TXT", HiddenContentPolicy::default())
        .await
        .expect("txt decoded");
    assert_eq!(document.format_id().as_str(), "elide.text.txt");
    assert_eq!(document.streams_mut().len(), 1);
    assert!(document.stream_mut::<Text>("body").is_some());
}

#[tokio::test]
async fn decode_content_resolves_from_filename() {
    let reg = FormatRegistry::with_builtin();
    let content = ContentData::from_text("hello\nworld\n").with_filename("notes.txt");
    let document = reg
        .decode_content(content, HiddenContentPolicy::default())
        .await
        .expect("resolved by filename");
    assert_eq!(document.format_id().as_str(), "elide.text.txt");
}

#[tokio::test]
async fn decode_content_resolves_from_content_type() {
    let reg = FormatRegistry::with_builtin();
    let content = ContentData::from_text("plain").with_content_type("text/plain");
    let document = reg
        .decode_content(content, HiddenContentPolicy::default())
        .await
        .expect("resolved by content type");
    assert_eq!(document.format_id().as_str(), "elide.text.txt");
}

/// An extension no format claims falls back to the content type.
#[tokio::test]
async fn decode_content_falls_back_to_the_content_type() {
    let reg = FormatRegistry::with_builtin();
    let content = ContentData::from_text("plain")
        .with_filename("notes.unknown")
        .with_content_type("text/plain");
    let document = reg
        .decode_content(content, HiddenContentPolicy::default())
        .await
        .expect("resolved by content type");
    assert_eq!(document.format_id().as_str(), "elide.text.txt");
}

#[tokio::test]
async fn decode_content_without_hints_is_unavailable() {
    let reg = FormatRegistry::with_builtin();
    let error = reg
        .decode_content(ContentData::from_text("x"), HiddenContentPolicy::default())
        .await
        .expect_err("no hints");
    assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
}

#[tokio::test]
async fn unknown_extension_is_unavailable() {
    let reg = FormatRegistry::with_builtin();
    let error = reg
        .decode("data", "xyz", HiddenContentPolicy::default())
        .await
        .expect_err("no xyz format");
    assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
}

#[tokio::test]
async fn decode_redact_reencode_round_trip() {
    let reg = FormatRegistry::with_builtin();
    let mut document = reg
        .decode(
            "contact alice@example.test today",
            "txt",
            HiddenContentPolicy::default(),
        )
        .await
        .expect("decoded");
    let mut stream = document.stream_mut::<Text>("body").expect("text stream");

    // "contact " is 8 bytes; "alice@example.test" is 18, so 8..26.
    stream
        .write_at(Redactions::from([(
            TextLocation::inline(8..26),
            TextReplacement::substituted("[EMAIL]"),
        )]))
        .await
        .expect("redacted");

    let out = document.encode().expect("re-encoded");
    assert_eq!(out.as_bytes(), b"contact [EMAIL] today");
}

#[cfg(feature = "csv")]
#[tokio::test]
async fn csv_decode_redact_reencode_round_trip() {
    use elide_core::modality::tabular::{Tabular, TabularLocation, TabularReplacement};

    let reg = FormatRegistry::with_builtin();
    let mut document = reg
        .decode(
            "name,email\nAlice,alice@x.test\n",
            "csv",
            HiddenContentPolicy::default(),
        )
        .await
        .expect("csv decoded");
    assert_eq!(document.format_id().as_str(), "elide.tabular.csv");
    let mut stream = document
        .stream_mut::<Tabular>("body")
        .expect("tabular stream");

    stream
        .write_at(Redactions::from([(
            TabularLocation::new(1, 1),
            TabularReplacement::Cell(TextReplacement::substituted("[EMAIL]")),
        )]))
        .await
        .expect("redacted");

    let out = document.encode().expect("re-encoded");
    assert_eq!(out.decode().unwrap(), "name,email\nAlice,[EMAIL]\n");
}
