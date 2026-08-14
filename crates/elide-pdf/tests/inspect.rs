//! Inspection: risk inventory, coverage, and fail-closed refusals over PDFs
//! built with lopdf.

use elide_pdf::Pdf;
use elide_pdf::inspect::{CoverageGap, CoverageStatus};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

/// A one-page PDF with a text layer, plus an `/Info` dictionary and one
/// annotation — enough to exercise the risk inventory.
fn rich_pdf() -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("Alice")]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

    // A link annotation on the page.
    let annot_id = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![72.into(), 700.into(), 200.into(), 712.into()],
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
        "Annots" => vec![annot_id.into()],
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);

    // Document information dictionary with two entries.
    let info_id = doc.add_object(dictionary! {
        "Author" => Object::string_literal("Alice Johnson"),
        "Title" => Object::string_literal("Onboarding"),
    });
    doc.trailer.set("Info", info_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn inventories_info_and_annotations() {
    let pdf = rich_pdf();
    let inspection = Pdf::open(&pdf).unwrap().inspect().unwrap();

    assert_eq!(inspection.page_count, 1);
    assert!(!inspection.encrypted);
    assert_eq!(inspection.risks.document_info_entry_count, 2);
    assert_eq!(inspection.risks.annotation_count, 1);
}

#[test]
fn a_clean_document_has_full_coverage() {
    let pdf = rich_pdf();
    let inspection = Pdf::open(&pdf).unwrap().inspect().unwrap();
    // A single-revision, unencrypted document is fully inspectable.
    assert_eq!(inspection.coverage.status, CoverageStatus::Full);
    assert!(inspection.coverage.gaps.is_empty());
}

#[test]
fn trailing_bytes_after_eof_are_a_retained_bytes_gap() {
    let mut pdf = rich_pdf();
    // Append non-whitespace content after the final %%EOF: retained bytes the
    // current object graph does not account for.
    pdf.extend_from_slice(b"\nleftover-sensitive-bytes");
    let inspection = Pdf::open(&pdf).unwrap().inspect().unwrap();

    assert!(inspection.risks.trailing_non_whitespace_byte_count > 0);
    assert_eq!(inspection.coverage.status, CoverageStatus::Partial);
    assert!(
        inspection
            .coverage
            .gaps
            .contains(&CoverageGap::RetainedDocumentBytes)
    );
}

#[test]
fn a_second_revision_is_an_incremental_revision() {
    use lopdf::IncrementalDocument;

    // Build a valid incremental update on top of the base document, so the
    // saved bytes carry two revisions (two `%%EOF` markers) and still parse.
    let base = rich_pdf();
    let prev = Document::load_mem(&base).unwrap();
    let mut incremental = IncrementalDocument::create_from(base, prev);
    // Add a new object so the update writes a real second revision.
    incremental
        .new_document
        .add_object(dictionary! { "Note" => Object::string_literal("second-revision") });
    let mut pdf = Vec::new();
    incremental.save_to(&mut pdf).unwrap();

    let inspection = Pdf::open(&pdf).unwrap().inspect().unwrap();
    assert!(inspection.risks.incremental_revision_count >= 1);
    assert_eq!(inspection.coverage.status, CoverageStatus::Partial);
    assert!(
        inspection
            .coverage
            .gaps
            .contains(&CoverageGap::RetainedDocumentBytes)
    );
}
