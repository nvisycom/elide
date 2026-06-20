//! End-to-end DOCX container round-trip: redact the body text through the
//! normal pipeline, drive the embedded image through the [`Orchestrator`],
//! then re-pack.
//!
//! Proves the full multi-modal path: the body XML (`word/document.xml`) is
//! analyzed and redacted as text, the `word/media/*` image is enumerated
//! as a container part, decoded through the registry, run through an
//! `Analyzer<Image>` + `Anonymizer<Image>`, and folded back into the
//! re-zipped package — while every structural part survives. The image
//! recognizer is the mock LLM backend (detects nothing), so the image
//! round-trips unchanged; the test asserts the path runs and the package
//! stays a valid DOCX.

mod fixtures;

use std::io::{Cursor, Read};

use elide::codec::{DocumentHandle, FormatRegistry};
use elide::entity::builtins;
use elide::modality::image::Image;
use elide::modality::text::Text;
use elide::primitive::{Language, LanguageTag};
use elide::recognition::Scope;
use elide::recognition::llm::LlmRecognizer;
use elide::{Analyzer, Anonymizer, Orchestrator, Result};
use fixtures::asserts::{assert_label_present, assert_pii_removed, assert_tokens_present};
use fixtures::pipeline::{build_analyzer, build_anonymizer};
use zip::ZipArchive;

const DOCX: &[u8] = include_bytes!("testdata/contact.docx");
const BODY_PART: &str = "word/document.xml";
const IMAGE_PART: &str = "word/media/image1.png";

/// An image analyzer backed by the mock LLM (detects nothing) — enough to
/// drive the orchestrator's image path without a real vision model.
fn image_analyzer() -> Result<Analyzer<Image>> {
    let recognizer = LlmRecognizer::<Image>::builder()
        .with_name("mock-image")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;
    Ok(Analyzer::new().with_recognizer(recognizer))
}

/// Read one entry's bytes out of a zip.
fn part(archive: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(archive.to_vec())).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

#[tokio::test]
async fn docx_redacts_body_and_drives_embedded_image() -> Result<()> {
    let registry = FormatRegistry::with_builtin();

    // Decode the DOCX as its text-backed body document.
    let mut docx: DocumentHandle<Text> = registry
        .decode(DOCX.to_vec(), "docx")
        .await?
        .into::<Text>()
        .expect("docx is text-backed");

    // Two phases: analyze the whole document (body text + embedded image),
    // inspect the body entities, then apply.
    let en = Language::asserted(LanguageTag::parse("en").unwrap());
    let orchestrator = Orchestrator::new(&registry)
        .with_modality::<Text>(
            build_analyzer::<Text>()?,
            build_anonymizer::<Text>(),
            Scope::<Text>::new().with_language(en),
        )
        .with_modality::<Image>(
            image_analyzer()?,
            Anonymizer::<Image>::new(),
            Scope::<Image>::new(),
        );

    let mut plan = orchestrator.analyze_document(&mut docx).await?;
    let body = plan.entities::<Text>().expect("body is text");
    for label in [
        builtins::EMAIL_ADDRESS.to_ref(),
        builtins::PHONE_NUMBER.to_ref(),
        builtins::PAYMENT_CARD.to_ref(),
        builtins::IBAN.to_ref(),
        builtins::GOVERNMENT_ID.to_ref(),
        builtins::IP_ADDRESS.to_ref(),
    ] {
        assert_label_present(body, &label);
    }

    orchestrator.apply(&mut docx, plan).await?;

    // Re-encode and inspect the rebuilt package.
    let out = docx.encode()?;
    let out = out.as_bytes();

    // The body XML: originals gone, replacement tokens in.
    let body = part(out, BODY_PART).expect("body present");
    let body = String::from_utf8(body).expect("body XML is UTF-8");
    assert_pii_removed(
        &body,
        &[
            "alice.johnson@example.com",
            "bob.smith@example.com",
            "+1 (415) 555-0142",
            "+1 (510) 555-0199",
            "4111 1111 1111 1111",
            "GB29 NWBK 6016 1331 9268 19",
            "123-45-6789",
            "192.168.1.42",
        ],
    );
    assert_tokens_present(
        &body,
        &[
            "[email_address]",
            "[phone_number]",
            "[iban]",
            "[government_id]",
            "[ip_address]",
        ],
    );

    // The embedded image survives the orchestrator round-trip as a valid
    // PNG, and the structural parts are still present.
    let image = part(out, IMAGE_PART).expect("image part present");
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n", "image part is not a PNG");
    assert!(
        part(out, "[Content_Types].xml").is_some(),
        "content-types part must survive",
    );
    assert!(
        part(out, "word/_rels/document.xml.rels").is_some(),
        "relationships part must survive",
    );

    Ok(())
}
