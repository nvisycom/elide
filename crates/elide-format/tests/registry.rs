//! Integration tests for [`FormatRegistry`]: resolve a document to a handler,
//! decode it, redact it, and re-encode it end to end through the built-in
//! formats.

#![cfg(feature = "txt")]

use elide_codec::DocumentPart;
use elide_codec::content::ContentData;
use elide_core::modality::DataWriter;
use elide_core::modality::text::{Text, TextLocation, TextReplacement};
use elide_core::redaction::Redactions;
use elide_format::FormatRegistry;

#[tokio::test]
async fn registry_decodes_txt_by_extension() {
    let reg = FormatRegistry::with_builtin();
    let document = reg
        .decode("hello\nworld\n", "txt")
        .await
        .expect("txt decoded");
    assert_eq!(document.format_id().as_str(), "elide.text.txt");
    // A leaf txt document is one text stream part.
    let DocumentPart::Stream { handle, .. } = &document.parts()[0] else {
        panic!("txt is a leaf stream document")
    };
    assert!(handle.is::<Text>());
}

#[tokio::test]
async fn decode_content_resolves_from_filename() {
    let reg = FormatRegistry::with_builtin();
    let content = ContentData::from_text("hello\nworld\n").with_filename("notes.txt");
    let handle = reg
        .decode_content(content)
        .await
        .expect("resolved by filename");
    assert_eq!(handle.format_id().as_str(), "elide.text.txt");
}

#[tokio::test]
async fn decode_content_resolves_from_content_type() {
    let reg = FormatRegistry::with_builtin();
    let content = ContentData::from_text("plain").with_content_type("text/plain");
    let handle = reg
        .decode_content(content)
        .await
        .expect("resolved by content type");
    assert_eq!(handle.format_id().as_str(), "elide.text.txt");
}

#[tokio::test]
async fn decode_content_without_hints_is_an_error() {
    let reg = FormatRegistry::with_builtin();
    assert!(
        reg.decode_content(ContentData::from_text("x"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stream_part_recovers_its_modality() {
    let reg = FormatRegistry::with_builtin();
    let mut document = reg.decode("hi", "txt").await.expect("decoded");
    assert_eq!(document.format_id().as_str(), "elide.text.txt");
    // The body stream recovers as Text; the TypeId downcast is exact.
    let DocumentPart::Stream { handle, .. } = &mut document.parts_mut()[0] else {
        panic!("txt is a leaf stream document")
    };
    assert!(handle.downcast_mut::<Text>().is_some());
}

#[tokio::test]
async fn decode_redact_reencode_round_trip() {
    let reg = FormatRegistry::with_builtin();
    let mut document = reg
        .decode("contact alice@example.test today", "txt")
        .await
        .expect("decoded");
    let DocumentPart::Stream { handle, .. } = &mut document.parts_mut()[0] else {
        panic!("txt is a leaf stream document")
    };
    let stream = handle.downcast_mut::<Text>().expect("text stream");

    let mut batch = Redactions::new();
    // "contact " is 8 bytes; "alice@example.test" is 18 → 8..26.
    batch.push(
        TextLocation::new(8, 26),
        TextReplacement::substituted("[EMAIL]"),
    );
    stream.write_at(batch).await.expect("redacted");

    let out = document.encode().expect("re-encoded");
    assert_eq!(out.as_bytes(), b"contact [EMAIL] today");
}

#[tokio::test]
async fn unknown_extension_is_an_error() {
    let reg = FormatRegistry::with_builtin();
    assert!(reg.decode("data", "xyz").await.is_err());
}

#[cfg(feature = "csv")]
#[tokio::test]
async fn registry_decodes_and_redacts_csv() {
    use elide_core::modality::StreamDataReader;
    use elide_core::modality::tabular::{Tabular, TabularLocation, TabularReplacement};

    let reg = FormatRegistry::with_builtin();
    let mut document = reg
        .decode("name,email\nAlice,alice@x.test\n", "csv")
        .await
        .expect("csv decoded");
    assert_eq!(document.format_id().as_str(), "elide.tabular.csv");
    let DocumentPart::Stream { handle, .. } = &mut document.parts_mut()[0] else {
        panic!("csv is a leaf stream document")
    };
    let stream = handle.downcast_mut::<Tabular>().expect("tabular stream");

    // Stream to the email cell, then redact it.
    let mut email_chunk = None;
    while let Some(chunk) = stream.read_next().await.expect("read") {
        if chunk.location.row_index == 1 && chunk.location.column_index == 1 {
            email_chunk = Some(chunk);
        }
    }
    assert!(email_chunk.is_some(), "found the email cell");

    let mut batch: Redactions<Tabular> = Redactions::new();
    batch.push(
        TabularLocation::new(1, 1),
        TabularReplacement::Cell(TextReplacement::substituted("[EMAIL]")),
    );
    stream.write_at(batch).await.expect("redacted");

    let out = document.encode().expect("re-encoded");
    assert_eq!(out.decode().unwrap(), "name,email\nAlice,[EMAIL]\n");
}
