//! End-to-end through the public facade: a DOCX's document properties
//! (`docProps/core.xml`) are detected and stripped alongside the body text,
//! driven by an `Orchestrator` a caller assembles from the re-exported
//! vocabulary.
//!
//! The `docx/body_header_hyperlink.docx` fixture carries `lastModifiedBy = "Oleh Martsokha"` in its
//! core properties. Under a hidden-content policy that redacts metadata, the
//! `Metadata` pipeline's `FieldRecognizer` flags that labelled field and `Erase`
//! removes it; the DOCX re-encode folds the cleared property part back into the
//! package, so the output no longer names the editor. Under the default policy
//! the property parts are removed whole at decode.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::LabelCatalog;
use elide::modality::metadata::Metadata;
use elide::modality::text::Text;
use elide::recognition::Scope;
use elide::recognition::metadata::FieldRecognizer;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Disposition, HiddenContentPolicy, Rule};
use elide::{Directives, Document, Orchestrator};
use elide_office::fixtures::read_part;

use super::FIXTURE;

/// The last-modified-by author the fixture carries in its core properties.
const EDITOR: &str = "Oleh Martsokha";

fn has_editor(docx: &[u8]) -> bool {
    read_part(docx, "docProps/core.xml")
        .map(|core| {
            elide_office::fixtures::property_fields(&core)
                .iter()
                .any(|(k, v)| k == "lastModifiedBy" && v == EDITOR)
        })
        .unwrap_or(false)
}

#[tokio::test]
async fn docx_core_properties_are_stripped_through_the_facade() {
    let original = bytes::Bytes::from_static(FIXTURE.source);
    assert!(has_editor(&original), "fixture should name the editor");

    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::Redact);
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        // Body text: an erase pipeline (any detected text redacts; not the focus).
        .with_modality::<Text>(
            Analyzer::new(),
            Anonymizer::new().with(Rule::fallback(Erase)),
        )
        // docProps: the field recognizer flags the labelled author/editor fields.
        .with_modality::<Metadata>(
            Analyzer::new().with_recognizer(FieldRecognizer),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "docx", policy)
        .await
        .expect("decode docx");
    let mut documents = [Document::new("body_header_hyperlink.docx", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await
        .expect("analyze");
    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");

    let out = documents[0].document.encode().expect("encode").to_bytes();
    assert!(!has_editor(&out), "editor survived the docProps strip");

    // The output must still be a valid DOCX package whose other parts survive:
    // the docProps edit must not corrupt the archive or drop the body.
    assert!(
        read_part(&out, "word/document.xml").is_some(),
        "body part missing after docProps strip"
    );
    // The part itself survives, but a property no field surfaces (the
    // revision count) is stripped at decode: nothing could have detected what
    // it holds.
    let core = read_part(&out, "docProps/core.xml").expect("core.xml survives");
    assert!(
        !std::str::from_utf8(&core).unwrap().contains("revision>"),
        "an unsurfaced core property shipped"
    );
}

#[tokio::test]
async fn docx_core_properties_are_removed_by_default() {
    let registry = FormatRegistry::with_builtin();
    let document = registry
        .decode(
            bytes::Bytes::from_static(FIXTURE.source),
            "docx",
            HiddenContentPolicy::default(),
        )
        .await
        .expect("decode docx");
    let out = document.encode().expect("encode").to_bytes();
    assert!(
        read_part(&out, "docProps/core.xml").is_none(),
        "core.xml survived"
    );
    assert!(
        read_part(&out, "word/document.xml").is_some(),
        "body part missing"
    );
}
