//! End-to-end through the public facade: an XLSX workbook's document-property
//! timestamps (`docProps/core.xml`) are detected and stripped, driven by an
//! `Orchestrator` a caller assembles from the re-exported vocabulary.
//!
//! Proves the OOXML `#docprops` sub-part model works for the tabular family too:
//! the `sample.xlsx` fixture carries populated `dcterms:created`/`modified`, the
//! `Metadata` pipeline's `DocPropsRecognizer` surfaces them, and `Erase` clears
//! them; the workbook re-encode folds the edited property part back in.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::LabelCatalog;
use elide::modality::metadata::Metadata;
use elide::modality::tabular::Tabular;
use elide::recognition::Scope;
use elide::recognition::docprops::DocPropsRecognizer;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Rule};
use elide::{Directives, Document, Orchestrator};
use elide_office::opc::props;
use elide_office::opc::test_util::read_part;

use super::FIXTURE;

/// Whether the workbook's core properties still carry a create/modify timestamp.
fn has_timestamp(xlsx: &[u8]) -> bool {
    read_part(xlsx, "docProps/core.xml")
        .map(|core| {
            props::fields(&core)
                .iter()
                .any(|(k, _)| k == "created" || k == "modified")
        })
        .unwrap_or(false)
}

#[tokio::test]
async fn xlsx_core_property_timestamps_are_stripped_through_the_facade() {
    let original = bytes::Bytes::from_static(FIXTURE.source);
    assert!(has_timestamp(&original), "fixture should carry timestamps");

    let registry = FormatRegistry::with_builtin();
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        // Cells: an erase pipeline (not the focus, but a registered body).
        .with_modality::<Tabular>(
            Analyzer::new(),
            Anonymizer::new().with(Rule::fallback(Erase)),
        )
        // docProps: the DocPropsRecognizer surfaces the timestamp fields.
        .with_modality::<Metadata>(
            Analyzer::new().with_recognizer(DocPropsRecognizer),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "xlsx")
        .await
        .expect("decode xlsx");
    let mut documents = [Document::new("sample.xlsx", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await
        .expect("analyze");
    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");

    let out = documents[0].handle.encode().expect("encode").to_bytes();
    assert!(
        !has_timestamp(&out),
        "timestamps survived the docProps strip"
    );
    // The workbook is still a valid package: its sole worksheet survives.
    assert!(
        read_part(&out, "xl/worksheets/sheet1.xml").is_some(),
        "worksheet missing after docProps strip"
    );
}
