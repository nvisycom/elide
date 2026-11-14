//! Applying a report fails closed: a report part with entities that cannot be
//! applied is an error, never a part silently left unredacted.

use elide_codec::mocks::{MOCK_EXT, encode_mock, mock_format};
use elide_core::entity::Entity;
use elide_core::entity::audit::{AuditEvent, PatternRule};
use elide_core::entity::builtins::EMAIL_ADDRESS;
use elide_core::modality::text::{Text, TextLocation};
use elide_core::modality::{Modality, NoArtifact};
use elide_core::primitive::Confidence;
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{ErrorKind, Result};
use elide_engine::{Document, Orchestrator, PartId, Report};
use elide_format::FormatRegistry;

/// A modality no orchestrator registers, sharing text's types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unregistered;

impl Modality for Unregistered {
    type Artifact = NoArtifact;
    type Data = <Text as Modality>::Data;
    type Location = <Text as Modality>::Location;
    type Replacement = <Text as Modality>::Replacement;

    const NAME: &'static str = "unregistered";
}

/// An entity of modality `M` over the first four bytes.
fn entity<M: Modality<Location = TextLocation>>() -> Entity<M> {
    let location = TextLocation::inline(0..4);
    let event = AuditEvent::pattern(
        "test",
        Confidence::MAX,
        location.clone(),
        PatternRule::default(),
    );
    Entity::new(EMAIL_ADDRESS.to_ref(), location).with_event(event)
}

/// An orchestrator over the mock format and one mock document named `doc.mock`.
async fn setup() -> Result<(Orchestrator, [Document; 1])> {
    let registry = FormatRegistry::new().with_format(mock_format());
    let handle = registry
        .decode(
            encode_mock("body text", &[]),
            MOCK_EXT,
            HiddenContentPolicy::default(),
        )
        .await?;
    let orchestrator = Orchestrator::new().with_registry(registry);
    Ok((orchestrator, [Document::new("doc.mock", handle)]))
}

#[tokio::test]
async fn a_part_with_no_pipeline_for_its_modality_is_an_error() -> Result<()> {
    let (orchestrator, mut documents) = setup().await?;
    let report =
        Report::new().insert_part::<Unregistered>(PartId::leaf("doc.mock"), vec![entity()]);
    let Err(error) = orchestrator.anonymize_with(&mut documents, report).await else {
        panic!("a part no pipeline can apply must fail");
    };
    assert_eq!(error.kind(), ErrorKind::MalformedInput);
    Ok(())
}

#[tokio::test]
async fn a_part_the_documents_do_not_have_is_an_error() -> Result<()> {
    let (orchestrator, mut documents) = setup().await?;
    let missing = PartId::leaf("doc.mock").child("missing.txt");
    let report = Report::new().insert_part::<Text>(missing, vec![entity()]);
    let Err(error) = orchestrator.anonymize_with(&mut documents, report).await else {
        panic!("a part the walk never reaches must fail");
    };
    assert_eq!(error.kind(), ErrorKind::MalformedInput);
    Ok(())
}

#[tokio::test]
async fn an_empty_part_that_cannot_be_applied_loses_nothing() -> Result<()> {
    let (orchestrator, mut documents) = setup().await?;
    let report = Report::new()
        .insert_part::<Unregistered>(PartId::leaf("doc.mock"), Vec::new())
        .insert_part::<Text>(PartId::leaf("doc.mock").child("missing.txt"), Vec::new());
    orchestrator.anonymize_with(&mut documents, report).await?;
    Ok(())
}
