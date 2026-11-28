//! A reviewer's mark on a PDF page region is redacted: the region resolves to
//! the glyphs it covers, which are deleted like a detection's. A region that
//! covers no text fails the apply rather than being skipped.

use std::num::NonZeroU32;

use elide::codec::FormatRegistry;
use elide::entity::{LabelRef, builtins};
use elide::modality::text::{Marked, Text, TextLocation};
use elide::primitive::UnitBoundingBox;
use elide::redaction::HiddenContentPolicy;
use elide::{ErrorKind, Orchestrator, PartId, RegistryDocumentExt, Report, Result};

use super::text::{FIXTURE, extracted_text};
use crate::support::orchestrator::{TestOrchestrator, build_anonymizer, default_text_analyzer};
use crate::support::review::included;

const DOC: &str = "doc.pdf";
const PHONE: &str = "555-0142";

fn orchestrator(registry: &FormatRegistry) -> Result<Orchestrator> {
    Ok(TestOrchestrator::bare()
        .with_registry(registry.clone())
        .with_text(default_text_analyzer()?, build_anonymizer::<Text>())
        .build())
}

/// A report holding one reviewer mark over `rect` on `page`.
fn marked(page: NonZeroU32, rect: UnitBoundingBox) -> Report {
    let mark = TextLocation::marked_rendered(page, rect);
    included(&PartId::from(DOC), builtins::PHONE_NUMBER.to_ref(), mark)
}

/// The page and first rectangle a detection of `label` is drawn in.
async fn drawn(
    orchestrator: &Orchestrator,
    label: LabelRef,
) -> Result<(NonZeroU32, UnitBoundingBox)> {
    let mut document = FormatRegistry::with_builtin()
        .document(DOC, FIXTURE.source, HiddenContentPolicy::default())
        .await?;
    let report = orchestrator.analyze(&mut document).await?.report;
    let part = report.part::<Text>(&document.id())?;
    let location = &part
        .entities()
        .iter()
        .find(|entity| *entity.label() == label)
        .expect("the label is detected")
        .location();
    Ok((location.page().expect("on a page"), location.rects()[0]))
}

#[tokio::test]
async fn a_marked_region_is_redacted() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;
    let (number, rect) = drawn(&orchestrator, builtins::PHONE_NUMBER.to_ref()).await?;

    // Apply only the mark, on a fresh document.
    let mut document = registry
        .document(DOC, FIXTURE.source, HiddenContentPolicy::default())
        .await?;
    let _applied = orchestrator
        .anonymize_with(&mut document, marked(number, rect))
        .await?;

    let redacted = extracted_text(document.document.encode()?.as_bytes()).await?;
    assert!(
        !redacted.contains(PHONE),
        "the marked number survived: {redacted}"
    );
    assert!(redacted.contains("alice.johnson@example.com"), "{redacted}");
    Ok(())
}

/// A mark over the detected number deletes only the number: the glyphs have
/// real widths, so the rest of its line lies outside the mark and survives.
#[tokio::test]
async fn a_marked_number_leaves_the_rest_of_its_line() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;
    let (number, rect) = drawn(&orchestrator, builtins::PHONE_NUMBER.to_ref()).await?;
    assert!(
        rect.width > 0.0 && rect.width < 0.5,
        "the number's box spans its glyphs, not its line: {rect:?}"
    );

    let mut document = registry
        .document(DOC, FIXTURE.source, HiddenContentPolicy::default())
        .await?;
    let _applied = orchestrator
        .anonymize_with(&mut document, marked(number, rect))
        .await?;

    let redacted = extracted_text(document.document.encode()?.as_bytes()).await?;
    assert!(!redacted.contains(PHONE), "{redacted}");
    assert!(redacted.contains("For urgent matters call"), "{redacted}");
    Ok(())
}

/// A mark of two regions with other text between them is refused: deleting
/// the glyphs it covers would leave a hole in the middle of the text.
#[tokio::test]
async fn two_regions_with_text_between_fail() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;
    let (number, rect) = drawn(&orchestrator, builtins::PHONE_NUMBER.to_ref()).await?;
    let slice = rect.width / 10.0;
    let left = UnitBoundingBox::new(rect.x, rect.y, slice, rect.height);
    let right = UnitBoundingBox::new(rect.x + rect.width - slice, rect.y, slice, rect.height);
    let mark = TextLocation::Marked(Marked::Rendered {
        page: number,
        rects: vec![left, right],
    });
    let report = included(&PartId::from(DOC), builtins::PHONE_NUMBER.to_ref(), mark);

    let mut document = registry
        .document(DOC, FIXTURE.source, HiddenContentPolicy::default())
        .await?;
    let Err(error) = orchestrator.anonymize_with(&mut document, report).await else {
        panic!("a mark with text between its regions must fail");
    };
    assert_eq!(error.kind(), ErrorKind::MalformedInput);
    Ok(())
}

#[tokio::test]
async fn a_region_with_no_text_fails() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let mut document = registry
        .document(DOC, FIXTURE.source, HiddenContentPolicy::default())
        .await?;
    let corner = UnitBoundingBox::new(0.995, 0.995, 0.005, 0.005);
    let Err(error) = orchestrator(&registry)?
        .anonymize_with(&mut document, marked(NonZeroU32::MIN, corner))
        .await
    else {
        panic!("a mark over no text must fail");
    };
    assert_eq!(error.kind(), ErrorKind::MalformedInput);
    Ok(())
}
