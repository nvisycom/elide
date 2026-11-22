//! End-to-end PDF **raster** redaction (feature `codec-pdf-render`): decode →
//! analyze → anonymize → encode, where every page is rendered, detected on
//! what it shows by the image pipeline, and replaced by that render with the
//! detected regions painted over.
//!
//! Requires the native PDFium library at runtime, so the test is `#[ignore]`d
//! by default; run it where PDFium is installed:
//!
//! ```sh
//! cargo test -p elide --features codec-pdf-render,image,mocks,engine,serde \
//!     --test format_pdf raster -- --ignored
//! ```
//!
//! Unlike glyph deletion, raster redaction never reads the text layer: it
//! paints over pixels. The proof is that the redacted output has **no
//! extractable text at all**, it is images, and none of the original PII
//! survives.

use bytes::Bytes;
use elide::codec::pdf::{Dpi, RasterMode, pdf_format_with};
use elide::codec::{ContentData, Document, FormatRegistry, LocalId};
use elide::modality::text::Text;
use elide::modality::{ChunkReader, DataWriter};
use elide::redaction::{HiddenContentPolicy, Redactions};
use elide::{ErrorKind, Orchestrator, RegistryDocumentExt, Result};
use elide_image::modality::{Image, ImageReplacement};
use elide_image::primitive::Color;
use elide_pdf::fixtures;

use crate::support::fixture::Fixture;
use crate::support::orchestrator::{TestOrchestrator, build_anonymizer, default_text_analyzer};

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

/// Read every page stream of `doc`, as detection reads it: a page replaced
/// by its render that nothing read is refused on encode.
fn read_pages(doc: &mut Document) -> Result<()> {
    for number in 1.. {
        let Some(page) = doc.stream_mut::<Image>(&format!("page-{number}")) else {
            return Ok(());
        };
        page.chunks()?;
    }
    Ok(())
}

/// The fixture redacted under `RasterMode::Always` by `orchestrator`, as bytes.
async fn redacted_always(orchestrator: &Orchestrator) -> Result<Bytes> {
    let registry =
        FormatRegistry::with_builtin().with_replaced_format(pdf_format_with(RasterMode::always()));
    let mut document = registry
        .document("doc.pdf", FIXTURE.source, HiddenContentPolicy::default())
        .await?;
    let report = orchestrator.analyze(&mut document).await?.report;
    orchestrator.anonymize_with(&mut document, report).await?;
    Ok(Bytes::copy_from_slice(
        document.document.encode()?.as_bytes(),
    ))
}

/// The total stream size of every image in `pdf`.
fn image_bytes(pdf: &[u8]) -> usize {
    fixtures::images(pdf).iter().map(Bytes::len).sum()
}

/// Without an image pipeline nothing detects on the renders that replace
/// every page, so the redaction is refused rather than ship them unredacted.
#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn always_without_an_image_pipeline_is_refused() -> Result<()> {
    let orchestrator = TestOrchestrator::bare()
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .build();
    let Err(error) = redacted_always(&orchestrator).await else {
        panic!("rendered pages nothing read must be refused");
    };
    assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
    Ok(())
}

/// With an image pipeline every page is detected on its render and replaced
/// by it: nothing is read from the text layer, and none survives.
#[cfg(feature = "image")]
#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn raster_redaction_flattens_every_page() -> Result<()> {
    use elide::detection::Analyzer;
    use elide::redaction::Anonymizer;

    let orchestrator = TestOrchestrator::bare()
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .with_image(Analyzer::new(), Anonymizer::new())
        .build();
    let redacted = redacted_always(&orchestrator).await?;

    // The redacted output is image-only: re-decoding recovers no text at all,
    // so no original PII can survive as text.
    let text = extracted_text(&redacted).await?;
    assert!(
        text.trim().is_empty(),
        "raster output must carry no text layer, got: {text:?}",
    );

    // And specifically none of the fixture's PII appears as bytes anywhere.
    let raw = String::from_utf8_lossy(&redacted);
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
    let mut doc = flattened(Bytes::from_static(FIXTURE.source), 72).await?;
    read_pages(&mut doc)?;
    let out = doc.encode()?;
    let text = extracted_text(out.as_bytes()).await?;
    assert!(text.trim().is_empty(), "a text layer survived: {text:?}");
    Ok(())
}

/// The configured resolution is the one pages are rendered at.
#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn pages_render_at_the_configured_dpi() -> Result<()> {
    let encoded = async |dpi| -> Result<ContentData> {
        let mut doc = flattened(Bytes::from_static(FIXTURE.source), dpi).await?;
        read_pages(&mut doc)?;
        doc.encode()
    };
    let (low, high) = (encoded(36).await?, encoded(144).await?);
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
    read_pages(&mut doc)?;
    let out = doc.encode()?;
    assert_eq!(
        fixtures::attachments(out.as_bytes()),
        [Bytes::from_static(b"[NAME]")]
    );
    Ok(())
}

/// A region redacted on a scanned page stays covered when an image on the
/// page is removed too, even where the box is the colour the page already
/// was: the region is painted over the page as it is then drawn.
#[tokio::test]
#[ignore = "requires the native PDFium library at runtime"]
async fn a_scanned_page_region_stays_covered_over_a_removed_image() -> Result<()> {
    let mut doc = pdf_format_with(RasterMode::Auto)
        .decode(
            ContentData::new(fixtures::scanned_image()),
            HiddenContentPolicy::default(),
        )
        .await?;
    let image: LocalId = doc
        .blobs()
        .iter()
        .find(|blob| blob.id.as_str().starts_with("img-"))
        .map(|blob| blob.id.clone())
        .expect("the image is surfaced");
    doc.remove_blob(&image)?;

    let mut page = doc
        .stream_mut::<Image>("page-1")
        .expect("the page is scanned");
    let frame = page.chunks()?.remove(0).location;
    let mut redactions = Redactions::new();
    let grey = Color::rgb(200, 200, 200);
    redactions.push(frame, ImageReplacement::Covered { color: grey });
    page.write_at(redactions).await?;

    let out = doc.encode()?;
    let samples = fixtures::image_samples(out.as_bytes());
    assert_eq!(samples.len(), 1, "the page is one image");
    assert!(
        samples[0].iter().all(|&sample| sample == 200),
        "the removed image's blank page shows through the box"
    );
    Ok(())
}
