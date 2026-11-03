//! End-to-end through the public facade: an XLSX workbook's document-property
//! timestamps (`docProps/core.xml`) are detected and stripped, driven by an
//! `Orchestrator` a caller assembles from the re-exported vocabulary.
//!
//! Proves the OOXML property-part metadata streams work for the tabular family
//! too: the `xlsx/shared_string_cells.xlsx` fixture carries populated
//! `dcterms:created`/`modified`, and under a hidden-content policy that treats
//! metadata as content the built-in metadata pipeline detects and erases them;
//! the workbook re-encode writes the edited property part back.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::LabelCatalog;
use elide::modality::tabular::Tabular;
use elide::recognition::Scope;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Disposition, HiddenContentPolicy, Rule};
use elide::{Directives, Document, Orchestrator};
use elide_office::fixtures::read_part;

use super::FIXTURE;

/// Whether the workbook's core properties still carry a create/modify timestamp.
fn has_timestamp(xlsx: &[u8]) -> bool {
    read_part(xlsx, "docProps/core.xml")
        .map(|core| {
            elide_office::fixtures::property_fields(&core)
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
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::TreatAsContent);
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        // Cells: an erase pipeline (not the focus, but a registered body).
        .with_modality::<Tabular>(
            Analyzer::new(),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "xlsx", policy)
        .await
        .expect("decode xlsx");
    let mut documents = [Document::new("shared_string_cells.xlsx", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await
        .expect("analyze");
    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");

    let out = documents[0].document.encode().expect("encode").to_bytes();
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
