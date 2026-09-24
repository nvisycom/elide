//! Magic-byte sniffing: [`FormatRegistry::decode_content`] infers a binary
//! format from the leading bytes when the content asserts no extension or
//! content type, and a caller-asserted hint always wins over the sniff.

#![cfg(all(feature = "sniff", feature = "png"))]

use elide_codec::content::ContentData;
use elide_format::FormatRegistry;

/// Raw PNG bytes with no filename and no content type resolve by their magic
/// number.
#[tokio::test]
async fn sniffs_a_png_from_raw_bytes() {
    let reg = FormatRegistry::with_builtin();
    let content = ContentData::new(elide_image::test_util::png(2, 2));
    let handle = reg.decode_content(content).await.expect("sniffed png");
    assert_eq!(handle.format_id().as_str(), "elide.image.png");
}

/// A caller-asserted extension wins over the sniff: PNG bytes labelled `.txt`
/// route to the text handler, not the image handler. The text handler then
/// rejects the non-UTF-8 bytes — proving the routing went to text (an image
/// route would have decoded the PNG cleanly).
#[tokio::test]
#[cfg(feature = "txt")]
async fn an_asserted_extension_beats_the_sniff() {
    use elide_core::ErrorKind;

    let reg = FormatRegistry::with_builtin();
    let content = ContentData::new(elide_image::test_util::png(2, 2)).with_filename("note.txt");
    let err = reg
        .decode_content(content)
        .await
        .expect_err("png bytes are not valid text");
    // The text handler's decode error, not a successful image decode: the `.txt`
    // extension beat the PNG magic bytes.
    assert_eq!(err.kind(), ErrorKind::MalformedInput);
}

/// Plain text has no magic bytes, so a sniff cannot resolve it: without a
/// filename or content type, it stays unresolvable.
#[tokio::test]
async fn plain_text_does_not_sniff() {
    let reg = FormatRegistry::with_builtin();
    let content = ContentData::from_text("just some words");
    assert!(reg.decode_content(content).await.is_err());
}

/// An unregistered but *present* hint suppresses the sniff: the caller asserted
/// a format (an extension we do not handle), so PNG bytes named `.unknown` fail
/// rather than being silently sniffed as an image.
#[tokio::test]
async fn an_unregistered_extension_suppresses_the_sniff() {
    let reg = FormatRegistry::with_builtin();
    let content = ContentData::new(elide_image::test_util::png(2, 2)).with_filename("data.unknown");
    assert!(
        reg.decode_content(content).await.is_err(),
        "a present-but-unregistered extension must not fall through to a sniff"
    );
}
