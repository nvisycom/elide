//! End-to-end through the public facade: a DOCX's document properties
//! (`docProps/core.xml`) are detected and stripped alongside the body text,
//! driven by an `Orchestrator` a caller assembles from the re-exported
//! vocabulary.
//!
//! The `sample.docx` fixture carries `lastModifiedBy = "Oleh Martsokha"` in its
//! core properties. The `Metadata` pipeline's `DocPropsRecognizer` surfaces that
//! field and `Erase` removes it; the DOCX re-encode folds the cleared property
//! part back into the package, so the output no longer names the editor.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::LabelCatalog;
use elide::modality::metadata::Metadata;
use elide::modality::text::Text;
use elide::recognition::Scope;
use elide::recognition::docprops::DocPropsRecognizer;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Document, Orchestrator};
use elide_office::opc::props;
use elide_office::opc::test_util::read_part;

use super::FIXTURE;

/// The last-modified-by author the fixture carries in its core properties.
const EDITOR: &str = "Oleh Martsokha";

fn has_editor(docx: &[u8]) -> bool {
    read_part(docx, "docProps/core.xml")
        .map(|core| {
            props::fields(&core)
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
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        // Body text: an erase pipeline (any detected text redacts; not the focus).
        .with_modality::<Text>(
            Analyzer::new(),
            Anonymizer::new().with(Rule::fallback(Erase)),
        )
        // docProps: the DocPropsRecognizer surfaces the author/editor fields.
        .with_modality::<Metadata>(
            Analyzer::new().with_recognizer(DocPropsRecognizer),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "docx")
        .await
        .expect("decode docx");
    let mut documents = [Document::new("sample.docx", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await
        .expect("analyze");
    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");

    let out = documents[0].handle.encode().expect("encode").to_bytes();
    assert!(!has_editor(&out), "editor survived the docProps strip");

    // The output must still be a valid DOCX package whose other parts survive:
    // the docProps edit must not corrupt the archive or drop the body.
    assert!(
        read_part(&out, "word/document.xml").is_some(),
        "body part missing after docProps strip"
    );
    // And a still-populated core property (the revision count) is preserved,
    // confirming the strip cleared only the picked fields.
    let core = read_part(&out, "docProps/core.xml").expect("core.xml survives");
    assert!(
        std::str::from_utf8(&core).unwrap().contains("<revision>"),
        "non-sensitive core properties were dropped"
    );
}
