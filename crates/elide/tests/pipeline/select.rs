//! The orchestrator's reviewable `select` seam: resolve the operator picks
//! for a whole document — body and parts — without redacting.
//!
//! [`select`] runs the anonymizer's rules over a report's detected entities and
//! hands back the picks (which operator hides which entity, and why) as a
//! [`DocumentSelections`] mirroring the report's body/parts shape. Each group is
//! an erased [`SelectionGroup`] a review layer inspects before anything is
//! applied: downcast it to the live picks for the in-process apply path, or take
//! [`views`] for the serializable, modality-free projection a review layer
//! ships. This exercises that seam over a real multi-part container.
//!
//! [`select`]: elide::Orchestrator::select
//! [`views`]: elide::SelectionGroup::views
//! [`DocumentSelections`]: elide::DocumentSelections
//! [`SelectionGroup`]: elide::SelectionGroup

use elide::codec::{FormatRegistry, PartId};
use elide::detection::Analyzer;
use elide::entity::builtins;
use elide::modality::image::Image;
use elide::modality::text::Text;
use elide::recognition::Scope;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::{Erase, Replace};
use elide::redaction::{Anonymizer, Rule, Selection};
use elide::{Directives, Orchestrator, Report, Result};

const SAMPLE: &[u8] = include_bytes!("../testdata/sample.docx");
const IMAGE_PART: &str = "word/media/image1.png";

/// Build an orchestrator whose body rule set is deterministic enough to read
/// back from the picks: email is replaced, everything else erased.
fn orchestrator(registry: &FormatRegistry) -> Result<Orchestrator<'_>> {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;
    let text = Anonymizer::new()
        .with(Rule::label(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[EMAIL]"),
        ))
        .with(Rule::fallback(Erase));
    let image = LlmRecognizer::<Image>::builder()
        .with_name("mock-image")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;
    Ok(Orchestrator::new(registry)
        .with_modality::<Text>(Analyzer::new().with_recognizer(patterns), text)
        .with_modality::<Image>(Analyzer::new().with_recognizer(image), Anonymizer::new()))
}

/// `select` resolves one pick per redaction over the body's entities, reading
/// no data and leaving the report untouched. Each pick names an operator from
/// the configured rules and covers at least one entity.
#[tokio::test]
async fn select_resolves_reviewable_body_picks() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    let mut doc = registry.decode(SAMPLE, "docx").await?;
    let report = orchestrator.analyze(&mut doc, &Directives::new()).await?;

    let selections = orchestrator.select(&report, &Scope::default());
    // The erased body group downcasts back to the body modality's selections.
    let group = selections
        .body
        .as_ref()
        .expect("body pipeline is registered");
    let body = group
        .as_any()
        .downcast_ref::<Vec<Selection<Text>>>()
        .expect("body selections are Text");
    assert!(!body.is_empty(), "the body resolves operator picks");

    for selection in body {
        let op = selection.operator_id();
        let name = op.name.as_str();
        assert!(
            name == "replace" || name == "erase",
            "each pick is one of the configured operators, got {name}",
        );
        assert!(!selection.entities().is_empty(), "a pick covers entities");
    }
    // The fixture's email addresses route to the replace rule.
    assert!(
        body.iter()
            .any(|s| s.operator_id().name.as_str() == "replace"),
        "the email rule fires on the fixture's addresses",
    );

    // The whole document projects to serializable, modality-free views without a
    // downcast — the seam a review layer ships over a wire.
    let views = selections.views();
    assert_eq!(
        views.len(),
        body.len(),
        "one view per body pick (no parts here)"
    );
    assert!(
        views
            .iter()
            .any(|v| v.operator_id.name.as_str() == "replace"),
        "views carry the same operator identities",
    );
    Ok(())
}

/// `select` routes each container part the report carries through its pipeline,
/// keying the picks by [`PartId`]. Built directly from a rebuilt report so the
/// part is present regardless of what the mock image backend detected.
#[tokio::test]
async fn select_resolves_container_parts() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    // A report carrying one image part with no detected entities — the pick
    // routes through the image pipeline (Anonymizer::new(), no rules → no
    // operator), so the group is present and empty.
    let image = PartId::new(IMAGE_PART);
    let report = Report::new().insert_part::<Image>(image.clone(), Vec::new());

    let selections = orchestrator.select(&report, &Scope::default());
    assert!(selections.body.is_none(), "the rebuilt report has no body");
    let group = selections
        .parts
        .get(&image)
        .expect("the image part routes to the image pipeline");
    let part = group
        .as_any()
        .downcast_ref::<Vec<Selection<Image>>>()
        .expect("image-part selections are Image");
    assert!(
        part.is_empty(),
        "no entities on the part → no picks, but the group is present",
    );
    Ok(())
}

/// `select` yields an empty [`DocumentSelections`] when nothing routes: no body
/// and no part with a registered pipeline.
#[tokio::test]
async fn select_is_empty_when_nothing_routes() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    // An unknown part modality never matched a pipeline; an empty report has no
    // body. Either way `select` returns an empty aggregate, not an error.
    let selections = orchestrator.select(&Report::new(), &Scope::default());
    assert!(
        selections.body.is_none(),
        "a report with no body has nothing to select"
    );
    assert!(selections.parts.is_empty(), "no parts, no part picks");
    assert!(
        selections.views().is_empty(),
        "an empty selection projects to no views"
    );
    Ok(())
}
