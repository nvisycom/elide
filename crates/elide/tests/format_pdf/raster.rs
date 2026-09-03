//! End-to-end PDF **raster** redaction (feature `codec-pdf-render`): decode →
//! analyze → anonymize → encode, where redaction fills the detected pixels and
//! emits a fresh image-only PDF.
//!
//! Requires the native PDFium library at runtime, so the test is `#[ignore]`d
//! by default; run it where PDFium is installed:
//!
//! ```sh
//! cargo test -p elide --features codec-pdf-render,test-utils,serde \
//!     --test e2e_pdf_raster -- --ignored
//! ```
//!
//! Unlike the text-splice path (`e2e_pdf`), raster redaction is reliable
//! regardless of font encoding: it never re-encodes text, it destroys pixels.
//! The proof is that the redacted output has **no extractable text at all**,
//! it is images, and none of the original PII survives.

use elide::Result;
use elide::codec::FormatRegistry;
use elide::codec::handler::pdf_format_with;
use elide::modality::StreamDataReader;
use elide::modality::text::Text;
use elide::primitive::RasterMode;

use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.pdf"),
    source: include_bytes!("../testdata/sample.pdf"),
    extension: "pdf",
};

/// Re-decode a PDF through the registry and concatenate its extracted text.
async fn extracted_text(pdf: &[u8]) -> Result<String> {
    let registry = FormatRegistry::with_builtin();
    let mut handle = registry
        .decode(pdf, "pdf")
        .await?
        .into::<Text>()
        .expect("pdf is text");
    let mut text = String::new();
    while let Some(chunk) = handle.read_next().await? {
        text.push_str(chunk.data.as_str());
    }
    Ok(text)
}

#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn raster_redaction_emits_a_sanitised_image_pdf() -> Result<()> {
    // Swap the built-in (glyph-delete) PDF handler for the raster one, which
    // flattens every page and emits a fresh image-only PDF.
    let registry =
        FormatRegistry::with_builtin().with_replaced_format(pdf_format_with(RasterMode::always()));
    let outcome = FIXTURE.run_with(registry).await?;

    // The born-digital text layer detected the PII (the analysis ran over the
    // observation text before redaction).
    assert!(
        !outcome.entities.is_empty(),
        "the born-digital text should detect entities",
    );

    // The redacted output is a fresh image-only PDF: re-decoding recovers no
    // text at all, so no original PII can survive as text.
    let text = extracted_text(&outcome.redacted).await?;
    assert!(
        text.trim().is_empty(),
        "raster output must carry no text layer, got: {text:?}",
    );

    // And specifically none of the fixture's PII appears as bytes anywhere.
    let raw = String::from_utf8_lossy(&outcome.redacted);
    for pii in [
        "alice.johnson@example.com",
        "555-0142",
        "4111 1111 1111 1111",
        "GB29 NWBK 6016 1331 9268 19",
        "123-45-6789",
        "192.168.1.42",
    ] {
        assert!(!raw.contains(pii), "PII survived in output: {pii}");
    }
    Ok(())
}
