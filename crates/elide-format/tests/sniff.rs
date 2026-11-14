//! Magic-byte sniffing: [`FormatRegistry::decode_content`] infers a binary
//! format from the leading bytes when the content asserts no extension or
//! content type, and an asserted one always wins over the sniff.

#![cfg(all(feature = "sniff", feature = "png", feature = "jpeg"))]

use elide_codec::content::ContentData;
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{ErrorKind, Result};
use elide_format::FormatRegistry;
use elide_image::fixtures::{jpeg_with_tags, png};

/// The format id `bytes` resolve to with no filename or content type.
async fn sniffed(bytes: impl Into<ContentData>) -> Result<&'static str> {
    let document = FormatRegistry::with_builtin()
        .decode_content(bytes.into(), HiddenContentPolicy::default())
        .await?;
    Ok(document.format_id().as_str())
}

#[tokio::test]
async fn sniffs_a_png() {
    assert_eq!(sniffed(png(2, 2)).await.unwrap(), "elide.image.png");
}

/// `infer` names JPEG `jpg`, one of the JPEG format's two extensions.
#[tokio::test]
async fn sniffs_a_jpeg() {
    let jpeg = jpeg_with_tags(false, &[]);
    assert_eq!(sniffed(jpeg).await.unwrap(), "elide.image.jpeg");
}

/// Plain text has no magic bytes, so without a filename or content type it
/// stays unresolvable.
#[tokio::test]
async fn plain_text_does_not_sniff() {
    let error = sniffed(ContentData::from_text("just some words"))
        .await
        .expect_err("text carries no magic bytes");
    assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
}

/// PNG bytes named `.txt` route to the text handler, which rejects them; an
/// image route would have decoded them.
#[tokio::test]
#[cfg(feature = "txt")]
async fn an_asserted_extension_beats_the_sniff() {
    let content = ContentData::new(png(2, 2)).with_filename("note.txt");
    let error = FormatRegistry::with_builtin()
        .decode_content(content, HiddenContentPolicy::default())
        .await
        .expect_err("png bytes are not valid text");
    assert_eq!(error.kind(), ErrorKind::MalformedInput);
}

/// A present but unregistered extension is a claim about the format, so PNG
/// bytes named `.unknown` fail rather than sniff as an image.
#[tokio::test]
async fn an_unregistered_extension_suppresses_the_sniff() {
    let content = ContentData::new(png(2, 2)).with_filename("data.unknown");
    let error = FormatRegistry::with_builtin()
        .decode_content(content, HiddenContentPolicy::default())
        .await
        .expect_err("no fall-through to the sniff");
    assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
}
