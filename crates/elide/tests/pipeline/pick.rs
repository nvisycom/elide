//! The operator *pick* recorded into each entity's audit trail.
//!
//! Redaction resolves which operator hides each entity (and why) and records
//! that decision as a [`Selection`] event on the entity's own audit trail,
//! alongside the [`Redaction`] that applies it, no parallel selection object.
//! A review layer reads the picks straight off the entities. This exercises
//! that over a real multi-part container: run [`anonymize`] and read the picks
//! back from the returned report's body entities.
//!
//! [`Selection`]: elide::entity::audit::AuditKind::Selection
//! [`Redaction`]: elide::entity::audit::AuditKind::Redaction
//! [`anonymize`]: elide::Orchestrator::anonymize

use elide::codec::FormatRegistry;
use elide::entity::audit::AuditKind;
use elide::modality::image::Image;
use elide::modality::text::Text;
use elide::redaction::HiddenContentPolicy;
use elide::{Orchestrator, PartId, RegistryDocumentExt, Report, Result};

use crate::support::orchestrator::TestOrchestrator;

const SAMPLE: &[u8] = include_bytes!("../testdata/docx/body_header_hyperlink.docx");

/// The fixture document's name, its depth-1 part key in the report.
const DOC: &str = "body_header_hyperlink.docx";

/// Build an orchestrator whose body rule set is deterministic enough to read
/// back from the picks: every detected label redacts through `Replace` (`[{label}]`).
fn orchestrator(registry: FormatRegistry) -> Result<Orchestrator> {
    Ok(TestOrchestrator::new()?.with_registry(registry).build())
}

/// `anonymize` records a reviewable pick on every body entity: each names one of
/// the configured operators, is followed by its redaction, and the whole trail
/// still verifies. The fixture's email addresses route to the replace rule.
#[tokio::test]
async fn anonymize_records_reviewable_body_picks() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(registry.clone())?;

    let mut doc = registry
        .document(DOC, SAMPLE, HiddenContentPolicy::default())
        .await?;
    let report = orchestrator.analyze(&mut doc).await?.report;
    let report = orchestrator.anonymize_with(&mut doc, report).await?;

    let body = report
        .part::<Text>(&doc.id())
        .ok()
        .map(|part| part.entities())
        .expect("the docx body is text and its pipeline is registered");
    assert!(!body.is_empty(), "the body detected entities to redact");

    for entity in body.iter() {
        let picked = entity
            .audit
            .selection()
            .expect("every redacted entity records its pick");
        let applied = entity
            .audit
            .applied_selection()
            .expect("the pick is followed by a redaction");
        let picked = picked.operator.name.as_str();
        assert!(
            picked == "replace" || picked == "erase" || picked == "mask",
            "each pick is one of the configured operators, got {picked}",
        );
        // The pick is recorded before it is applied, and the operator actually
        // run is the one that was picked.
        let selection_at = entity
            .audit
            .position(|k| matches!(k, AuditKind::Selection(_)));
        let redaction_at = entity
            .audit
            .position(|k| matches!(k, AuditKind::Redaction(_)));
        assert!(
            selection_at < redaction_at,
            "the Selection precedes the Redaction",
        );
        assert_eq!(
            picked, applied.operator.name,
            "the redaction ran the picked operator"
        );
        assert!(entity.audit.verify().is_ok(), "the trail still verifies");
    }
    // The fixture's email addresses route to the replace rule.
    assert!(
        body.iter()
            .any(|e| e.audit.selection().map(|s| s.operator.name.as_str()) == Some("replace")),
        "the email rule fires on the fixture's addresses",
    );
    Ok(())
}

/// `pick` previews the plan before anything is applied: every body entity
/// records the operator it will get, and applying afterwards runs exactly that
/// operator without recording a second, identical pick.
#[tokio::test]
async fn pick_previews_the_plan_anonymize_with_follows() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(registry.clone())?;

    let mut doc = registry
        .document(DOC, SAMPLE, HiddenContentPolicy::default())
        .await?;
    let mut report = orchestrator.analyze(&mut doc).await?.report;
    orchestrator.pick(&mut report)?;

    let planned: Vec<(uuid::Uuid, String)> = report
        .part::<Text>(&doc.id())?
        .entities()
        .iter()
        .map(|entity| {
            let selection = entity.audit.selection().expect("every entity is planned");
            assert!(entity.audit.redaction().is_none(), "nothing is applied yet");
            (entity.id, selection.operator.name.to_string())
        })
        .collect();
    assert!(!planned.is_empty(), "the body detected entities to plan");

    // Picking again records nothing new.
    let events = |report: &Report| -> usize {
        report
            .part::<Text>(&doc.id())
            .map(|part| part.entities().iter().map(|e| e.audit.events().len()).sum())
            .unwrap_or_default()
    };
    let before = events(&report);
    orchestrator.pick(&mut report)?;
    assert_eq!(events(&report), before, "a repeated pick adds nothing");

    let applied = orchestrator.anonymize_with(&mut doc, report).await?;
    let body = applied.part::<Text>(&doc.id())?;
    for (id, operator) in planned {
        let entity = body.entity(id).expect("the planned entity is still there");
        let selections = entity
            .audit
            .events()
            .iter()
            .filter(|event| matches!(event.kind, AuditKind::Selection(_)))
            .count();
        assert_eq!(selections, 1, "applying does not repeat the pick");
        let applied = entity.audit.applied_selection().expect("applied");
        assert_eq!(
            applied.operator.name.as_str(),
            operator,
            "the planned operator ran"
        );
    }
    Ok(())
}

/// A part with no detected entities routes through its pipeline and records no
/// picks, nothing to redact, no error. Built from a rebuilt report so the part
/// is present regardless of what the mock image backend detected.
#[tokio::test]
async fn anonymize_over_an_empty_part_records_nothing() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(registry.clone())?;

    let mut doc = registry
        .document(DOC, SAMPLE, HiddenContentPolicy::default())
        .await?;
    // A report carrying one image part with no detected entities, apply routes
    // through the image pipeline (Anonymizer::new(), no rules) and does nothing.
    // The part is keyed under the document's name (a nested path) so apply
    // re-decodes it from that document.
    let image = PartId::from(DOC).child("word/media/image1.png");
    let report = Report::new().insert_part::<Image>(image.clone(), Vec::new());

    let report = orchestrator.anonymize_with(&mut doc, report).await?;
    assert!(
        report.part_ids().all(|(id, _)| id.depth() > 1),
        "the report carries no document own-content, only the nested image part",
    );
    let part = report
        .part::<Image>(&image)
        .ok()
        .map(|part| part.entities())
        .expect("the image part routes to the image pipeline");
    assert!(part.is_empty(), "no entities on the part → no picks");
    Ok(())
}
