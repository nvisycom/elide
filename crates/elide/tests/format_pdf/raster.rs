//! End-to-end PDF **raster** redaction (feature `codec-pdf-render`): decode →
//! analyze → anonymize → encode, where every page is rendered with the
//! detected text painted over and replaced by that image.
//!
//! Requires the native PDFium library at runtime, so the test is `#[ignore]`d
//! by default; run it where PDFium is installed:
//!
//! ```sh
//! cargo test -p elide --features codec-pdf-render,mocks,serde \
//!     --test e2e_pdf_raster -- --ignored
//! ```
//!
//! Unlike glyph deletion, raster redaction does not depend on the font
//! encoding: it paints over pixels. The proof is that the redacted output has
//! **no extractable text at all**, it is images, and none of the original PII
//! survives.

use bytes::Bytes;
use elide::Result;
use elide::codec::pdf::{Dpi, RasterMode, pdf_format_with};
use elide::codec::{ContentData, Document, FormatRegistry, LocalId};
use elide::modality::ChunkReader;
use elide::modality::text::Text;
use elide::redaction::HiddenContentPolicy;
use elide_pdf::fixtures;

use crate::support::fixture::Fixture;

const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/pdf/born_digital_text.pdf"
    ),
    source: include_bytes!("../testdata/pdf/born_digital_text.pdf"),
    extension: "pdf",
};

/// Re-decode a PDF through the registry and concatenate its extracted text.
async fn extracted_text(pdf: &[u8]) -> Result<String> {
    let registry = FormatRegistry::with_builtin();
    let mut document = registry
        .decode(pdf, "pdf", HiddenContentPolicy::default())
        .await?;
    let stream = document.stream_mut::<Text>("body").expect("pdf is text");
    let mut text = String::new();
    for chunk in stream.chunks()? {
        text.push_str(chunk.data.as_str());
    }
    Ok(text)
}

/// Decode `pdf` under `RasterMode::Always` at `dpi`.
async fn flattened(pdf: Bytes, dpi: u16) -> Result<Document> {
    pdf_format_with(RasterMode::always_at(Dpi::new(dpi)))
        .decode(ContentData::new(pdf), HiddenContentPolicy::default())
        .await
}

/// The total stream size of every image in `pdf`.
fn image_bytes(pdf: &[u8]) -> usize {
    fixtures::images(pdf).iter().map(Bytes::len).sum()
}

#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn raster_redaction_flattens_every_page() -> Result<()> {
    // Swap the built-in (glyph-delete) PDF handler for the raster one, which
    // replaces every page by a redacted image of it.
    let registry =
        FormatRegistry::with_builtin().with_replaced_format(pdf_format_with(RasterMode::always()));
    let outcome = FIXTURE.run_with(registry).await?;

    // The born-digital text layer detected the PII (the analysis ran over the
    // observation text before redaction).
    assert!(
        !outcome.entities.is_empty(),
        "the born-digital text should detect entities",
    );

    // The redacted output is image-only: re-decoding recovers no text at all,
    // so no original PII can survive as text.
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

/// A page with nothing detected is flattened too: no text layer survives.
#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn every_page_is_flattened_with_nothing_detected() -> Result<()> {
    let doc = flattened(Bytes::from_static(FIXTURE.source), 72).await?;
    let out = doc.encode()?;
    let text = extracted_text(out.as_bytes()).await?;
    assert!(text.trim().is_empty(), "a text layer survived: {text:?}");
    Ok(())
}

/// The configured resolution is the one pages are rendered at.
#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn pages_render_at_the_configured_dpi() -> Result<()> {
    let low = flattened(Bytes::from_static(FIXTURE.source), 36)
        .await?
        .encode()?;
    let high = flattened(Bytes::from_static(FIXTURE.source), 144)
        .await?
        .encode()?;
    assert!(
        image_bytes(high.as_bytes()) > image_bytes(low.as_bytes()),
        "a higher DPI did not render more pixels"
    );
    Ok(())
}

/// Embedded images and attachments are surfaced for their own redaction, and
/// a redacted attachment is written back.
#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn images_and_attachments_are_surfaced() -> Result<()> {
    let doc = flattened(fixtures::raw_image(), 72).await?;
    assert!(
        doc.blobs()
            .iter()
            .any(|blob| blob.id.as_str().starts_with("img-"))
    );

    let mut doc = flattened(fixtures::metadata_and_attachment(), 72).await?;
    let id: LocalId = doc
        .blobs()
        .iter()
        .find(|blob| blob.id.as_str().starts_with("att-"))
        .map(|blob| blob.id.clone())
        .expect("the attachment is surfaced");
    doc.replace_blob(&id, Bytes::from_static(b"[NAME]"))?;
    let out = doc.encode()?;
    assert_eq!(
        fixtures::attachments(out.as_bytes()),
        [Bytes::from_static(b"[NAME]")]
    );
    Ok(())
}
