//! A set of loose files redacted as one logical document: each file is a
//! top-level part keyed by its name, analyzed and applied on its own handle. A
//! set has no single body — every file, container or leaf, is a part. This
//! exercises the multi-document entry point and the `PartId` path model that
//! keeps two files' parts distinct.

#![cfg(all(
    feature = "engine",
    feature = "codec-png",
    feature = "codec-docx",
    feature = "test-utils",
    feature = "llm",
    feature = "ocr",
))]

use elide::codec::FormatRegistry;
use elide::enrichment::ocr::MockBackend;
use elide::modality::image::{Image, ImageLocation, LayoutBlock};
use elide::modality::text::Text;
use elide::primitive::{BoundingBox, Point};
use elide::{Directives, PartId, RegistryDocumentExt, Result};

use crate::support::orchestrator::{
    TestOrchestrator, build_anonymizer, default_text_analyzer, erase_anonymizer, ocr_analyzer,
};

/// A real (small) PNG the image codec decodes.
const SAMPLE_PNG: &[u8] = include_bytes!("../testdata/sample.png");

/// An orchestrator that redacts inside images: OCR text carrying a detectable
/// email, a pattern recognizer over it, an erase anonymizer.
fn orchestrator(registry: FormatRegistry) -> Result<elide::Orchestrator> {
    let block = LayoutBlock::new(
        ImageLocation::new(BoundingBox::from_origin_size(
            Point::new(0.0, 0.0),
            200.0,
            20.0,
        )),
        "write to alice@example.com today",
    );
    Ok(TestOrchestrator::bare()
        .with_registry(registry)
        .with_image(
            ocr_analyzer(MockBackend::with(vec![block]))?,
            erase_anonymizer(),
        )
        .build())
}

/// A set of files with no wrapping archive is redacted as one logical document:
/// each file is a top-level part keyed by its name, analyzed and applied on its
/// own handle. Two plain image files (no container in sight) prove the
/// files-are-parts entry point — a set has *no* single body.
#[tokio::test]
async fn a_set_treats_each_file_as_a_named_part() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(registry.clone())?;

    // Two standalone images, supplied side by side.
    let mut documents = [
        registry.document("scan-A.png", SAMPLE_PNG).await?,
        registry.document("scan-B.png", SAMPLE_PNG).await?,
    ];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await?;

    // No single body: every file is a depth-1 part keyed by name, so a set of
    // two has two depth-1 parts (not one privileged body).
    let tops: Vec<_> = analyzed
        .report
        .part_ids()
        .filter(|(id, _)| id.depth() == 1)
        .map(|(id, _)| id.to_string())
        .collect();
    assert_eq!(
        tops.len(),
        2,
        "each file is its own depth-1 part; got: {tops:?}"
    );
    assert!(tops.contains(&"scan-A.png".to_string()), "got: {tops:?}");
    assert!(tops.contains(&"scan-B.png".to_string()), "got: {tops:?}");
    for name in ["scan-A.png", "scan-B.png"] {
        let id = PartId::new(name);
        assert!(
            analyzed.artifacts.part::<Image>(&id).is_some(),
            "the OCR layout for {name} was surfaced",
        );
    }

    // Applying redacts each file in place; each re-encodes on its own.
    let report = orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await?;
    assert!(!report.part_ids().collect::<Vec<_>>().is_empty());
    for file in &documents {
        assert!(
            !file.handle.encode()?.as_bytes().is_empty(),
            "each file re-encodes to non-empty bytes",
        );
    }
    Ok(())
}

/// A file in the set whose own BODY carries PII (a DOCX's `word/document.xml`)
/// is redacted, keyed by the file's name — proving each file's own content (not
/// only its embedded media) is analyzed, and that two files in one set are each
/// reached and redacted.
#[tokio::test]
async fn a_set_redacts_each_files_own_content() -> Result<()> {
    use elide::modality::text::Text;

    const SAMPLE_DOCX: &[u8] = include_bytes!("../testdata/sample.docx");

    let registry = FormatRegistry::with_builtin();
    let orchestrator = TestOrchestrator::bare()
        .with_registry(registry.clone())
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .build();

    // Two DOCX files supplied together; each carries an email in its body.
    let mut documents = [
        registry.document("first.docx", SAMPLE_DOCX).await?,
        registry.document("second.docx", SAMPLE_DOCX).await?,
    ];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await?;

    for name in ["first.docx", "second.docx"] {
        let id = PartId::new(name);
        let entities = analyzed
            .report
            .part_entities::<Text>(&id)
            .unwrap_or_else(|| panic!("{name}'s body reconstructs as Text"));
        assert!(
            !entities.is_empty(),
            "{name}'s body PII (an email) was detected — not leaked",
        );
    }

    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await?;
    // Each output re-encodes AND its body no longer carries the email: decode
    // `word/document.xml` from each package and assert the PII is gone (it was
    // `[email_address]`-replaced), so both named documents are independently
    // redacted — not merely re-encoded unchanged.
    for file in &documents {
        let out = file.handle.encode()?;
        let body = elide_office::opc::test_util::read_part(out.as_bytes(), "word/document.xml")
            .expect("output has a word/document.xml part");
        let body = String::from_utf8(body).expect("body is UTF-8");
        assert!(
            !body.contains("bob.smith@example.com"),
            "{}'s body email must be redacted, got: {body}",
            file.name,
        );
        assert!(
            body.contains("[email_address]"),
            "the replacement landed in the body"
        );
    }
    Ok(())
}

/// A nested container whose OWN body carries PII *and* which contains a
/// descendant that also carries PII has BOTH redacted — the container's own
/// redaction is not clobbered by re-decoding the original when its descendants
/// are folded. Regression for the fold-order data-loss bug.
///
/// The fixture `testdata/docx/nested.docx` is three DOCX nested by embedding:
/// `outer.docx` (body `alice@example.com`) → `word/embeddings/middle.docx`
/// (body `bob@example.com`) → `word/embeddings/leaf.docx` (body
/// `carol@example.com`), so each container level has its own body PII at a known
/// depth.
#[tokio::test]
async fn a_nested_container_keeps_its_own_redaction_when_a_descendant_folds() -> Result<()> {
    const NESTED_DOCX: &[u8] = include_bytes!("../testdata/docx/nested.docx");

    let registry = FormatRegistry::with_builtin();
    let orchestrator = TestOrchestrator::bare()
        .with_registry(registry.clone())
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .build();
    let mut document = registry.document("outer.docx", NESTED_DOCX).await?;

    let analyzed = orchestrator
        .analyze(&mut document, &Directives::new())
        .await?;
    // All three bodies are reached: outer (depth 1), middle (depth 2, a
    // container with its own body), and leaf (depth 3).
    let depths: Vec<usize> = analyzed
        .report
        .part_ids()
        .map(|(id, _)| id.depth())
        .collect();
    assert!(
        depths.contains(&1) && depths.contains(&2) && depths.contains(&3),
        "every nesting level's body is analyzed; got depths {depths:?}",
    );

    orchestrator
        .anonymize_with(&mut document, analyzed.report)
        .await?;

    // Decode the output tree and assert NO body email survives at any level —
    // in particular middle.docx's own `bob@example.com`, which the fold bug drops.
    let out = document.handle.encode()?;
    let out_bytes = out.as_bytes().to_vec();
    let read = |pkg: &[u8], part: &str| {
        elide_office::opc::test_util::read_part(pkg, part)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    };
    let outer_body = read(&out_bytes, "word/document.xml").expect("outer body");
    assert!(
        !outer_body.contains("alice@example.com"),
        "outer body: {outer_body}"
    );

    let middle_bytes =
        elide_office::opc::test_util::read_part(&out_bytes, "word/embeddings/middle.docx")
            .expect("middle embedded");
    let middle_body = read(&middle_bytes, "word/document.xml").expect("middle body");
    assert!(
        !middle_body.contains("bob@example.com"),
        "the nested container's OWN body redaction was lost: {middle_body}",
    );

    let leaf_bytes =
        elide_office::opc::test_util::read_part(&middle_bytes, "word/embeddings/leaf.docx")
            .expect("leaf embedded in middle");
    let leaf_body = read(&leaf_bytes, "word/document.xml").expect("leaf body");
    assert!(
        !leaf_body.contains("carol@example.com"),
        "leaf body: {leaf_body}"
    );
    Ok(())
}

/// Two documents sharing a name would collide on their depth-1 `PartId` — the
/// second silently overwriting the first — so `analyze` rejects the set up front
/// rather than dropping a document's redaction.
#[tokio::test]
async fn duplicate_document_names_are_rejected() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = TestOrchestrator::bare()
        .with_registry(registry.clone())
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .build();

    let mut documents = [
        registry.document("scan.png", SAMPLE_PNG).await?,
        registry.document("scan.png", SAMPLE_PNG).await?,
    ];
    let err = match orchestrator
        .analyze(&mut documents, &Directives::new())
        .await
    {
        Ok(_) => panic!("duplicate document names must be rejected"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("duplicate document name"),
        "got: {err}",
    );
    Ok(())
}
