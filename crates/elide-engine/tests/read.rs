//! Reading the data at entities' locations with `Orchestrator::read`: what each
//! entity covers, at every nesting level, before redaction changes it.
//!
//! Runs through the mock container codec, whose body is text and whose parts
//! nest.

use elide_codec::mocks::{MOCK_EXT, MockPart, encode_mock, mock_format};
use elide_core::entity::audit::{Pattern, PatternRule};
use elide_core::entity::builtins::EMAIL_ADDRESS;
use elide_core::entity::{Birth, Entity, LabelCatalog};
use elide_core::mocks::MockOperator;
use elide_core::modality::text::{Text, TextLocation};
use elide_core::primitive::{ComponentId, Confidence};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Component, ErrorKind, Result};
use elide_detection::Analyzer;
use elide_engine::{Document, Orchestrator, PartId};
use elide_format::FormatRegistry;
use elide_redaction::{Anonymizer, Rule};

/// The PII the recognizer flags.
const PII: &str = "secret@example.com";

/// Flags every occurrence of [`PII`] in the chunk text.
#[derive(Component)]
#[component(name = "mock-pii", version = "1")]
struct PiiRecognizer;

#[async_trait::async_trait]
impl Recognizer<Text> for PiiRecognizer {
    async fn recognize(
        &self,
        subject: &Subject<Text>,
        _ctx: &Context<'_>,
    ) -> Result<Vec<Entity<Text>>> {
        let text = subject.data().as_str();
        Ok(text
            .match_indices(PII)
            .map(|(start, _)| {
                let location = TextLocation::inline(start..start + PII.len());
                Entity::new(Birth::new(
                    Confidence::MAX,
                    Pattern::new(
                        ComponentId::new("mock-pii", "1"),
                        EMAIL_ADDRESS.to_ref(),
                        location,
                        PatternRule::default(),
                    ),
                ))
            })
            .collect())
    }
}

/// An orchestrator over the mock codec, finding [`PII`] in text.
fn orchestrator(registry: FormatRegistry) -> Orchestrator {
    Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry)
        .with_modality::<Text>(
            Analyzer::new().with_recognizer(PiiRecognizer),
            Anonymizer::new().with(Rule::fallback(MockOperator::new("[REDACTED]"))),
        )
}

/// A mock container whose body holds [`PII`], with `parts` embedded in it.
fn container(level: &str, parts: Vec<MockPart>) -> bytes::Bytes {
    encode_mock(&format!("{level} body holds {PII} here"), &parts)
}

/// Every text entity of a nested document reads back as the text it covers,
/// at every level, and nothing is changed by the read.
#[tokio::test]
async fn entities_read_back_as_what_they_cover_at_every_level() -> Result<()> {
    let leaf = container("leaf", Vec::new());
    let outer = container(
        "outer",
        vec![MockPart {
            id: "leaf".to_owned(),
            hint: MOCK_EXT.to_owned(),
            bytes: leaf,
        }],
    );
    let registry = FormatRegistry::new().with_format(mock_format());
    let orchestrator = orchestrator(registry.clone());
    let handle = registry
        .decode(outer, MOCK_EXT, HiddenContentPolicy::default())
        .await?;
    let mut documents = [Document::new("outer.mock", handle)];

    let analyzed = orchestrator.analyze(&mut documents).await?;
    let mut locations: Vec<(PartId, TextLocation)> = Vec::new();
    let ids: Vec<PartId> = analyzed
        .report
        .part_ids()
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        for entity in analyzed.report.part::<Text>(&id)?.entities() {
            locations.push((id.clone(), entity.location().clone()));
        }
    }
    let depths: Vec<usize> = locations.iter().map(|(id, _)| id.depth()).collect();
    assert!(depths.contains(&1) && depths.contains(&2), "{depths:?}");

    let data = orchestrator
        .read::<Text>(&mut documents, &locations)
        .await?;
    assert!(
        data.iter()
            .all(|data| data.as_ref().is_ok_and(|data| data.as_str() == PII)),
        "{data:?}"
    );

    // The read changed nothing: redaction still finds and replaces every one.
    let report = orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await?;
    assert_eq!(report.part_ids().count(), depths.len());
    Ok(())
}

/// A location in a part the documents do not have is not found, and one
/// beyond its part's text is malformed, each failing on its own while the
/// readable location beside them is read.
#[tokio::test]
async fn an_unreadable_location_fails_on_its_own() -> Result<()> {
    let registry = FormatRegistry::new().with_format(mock_format());
    let orchestrator = orchestrator(registry.clone());
    let handle = registry
        .decode(
            container("outer", Vec::new()),
            MOCK_EXT,
            HiddenContentPolicy::default(),
        )
        .await?;
    let mut documents = [Document::new("outer.mock", handle)];
    let outer = PartId::leaf("outer.mock");
    let locations = [
        (outer.child("absent"), TextLocation::inline(0..1)),
        (outer.clone(), TextLocation::inline(0..9_999)),
        (outer, TextLocation::inline(0..5)),
    ];
    let reads = orchestrator
        .read::<Text>(&mut documents, &locations)
        .await?;
    assert_eq!(reads[0].as_ref().unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(
        reads[1].as_ref().unwrap_err().kind(),
        ErrorKind::MalformedInput
    );
    assert_eq!(
        reads[2].as_ref().map(|data| data.as_str()).ok(),
        Some("outer")
    );
    Ok(())
}
