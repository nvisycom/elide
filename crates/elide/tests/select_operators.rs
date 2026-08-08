//! The orchestrator's reviewable `select` seam: resolve the operator picks
//! for a document's body and parts without redacting.
//!
//! `select_body` / `select_part` run the anonymizer's rules over a report's
//! detected entities and hand back the picks (which operator hides which
//! entity, and why) as an erased [`SelectionGroup`] — the artifact a review
//! layer downcasts, inspects, and serializes before anything is applied. This
//! exercises that seam over a real multi-part container.
//!
//! [`SelectionGroup`]: elide::SelectionGroup

mod fixtures;

use elide::codec::{FormatRegistry, PartId};
use elide::detection::Analyzer;
use elide::entity::builtins;
use elide::modality::image::Image;
use elide::modality::text::Text;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::{Erase, Replace};
use elide::redaction::{Anonymizer, Rule, Selection};
use elide::recognition::Scope;
use elide::{Directives, Orchestrator, Report, Result};

const SAMPLE: &[u8] = include_bytes!("testdata/sample.docx");
const IMAGE_PART: &str = "word/media/image1.png";

/// Build an orchestrator whose body rule set is deterministic enough to read
/// back from the picks: email is replaced, everything else erased.
fn orchestrator(registry: &FormatRegistry) -> Result<Orchestrator<'_>> {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;
    let text = Anonymizer::new()
        .with(Rule::label(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]")))
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

/// `select_body` resolves one pick per redaction over the body's entities,
/// reading no data and leaving the report untouched. Each pick names an
/// operator from the configured rules and covers at least one entity.
#[tokio::test]
async fn select_body_resolves_reviewable_picks() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    let mut doc = registry.decode(SAMPLE, "docx").await?;
    let report = orchestrator.analyze(&mut doc, &Directives::new()).await?;

    // The erased group downcasts back to the body modality's selections.
    let group = orchestrator
        .select_body(&report, &Scope::default())
        .expect("body pipeline is registered");
    let selections = group
        .as_any()
        .downcast_ref::<Vec<Selection<Text>>>()
        .expect("body selections are Text");
    assert!(!selections.is_empty(), "the body resolves operator picks");

    for selection in selections {
        let op = selection.operator_id().name.as_str();
        assert!(
            op == "replace" || op == "erase",
            "each pick is one of the configured operators, got {op}",
        );
        assert!(!selection.entities().is_empty(), "a pick covers entities");
    }
    // The fixture's email addresses route to the replace rule.
    assert!(
        selections
            .iter()
            .any(|s| s.operator_id().name.as_str() == "replace"),
        "the email rule fires on the fixture's addresses",
    );
    Ok(())
}

/// `select_part` routes through the pipeline of any part the report carries,
/// returning that part's picks as an `Image` selection group. Built directly
/// from a rebuilt report so the part is present regardless of what the mock
/// image backend detected during analysis.
#[tokio::test]
async fn select_part_resolves_a_container_part() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    // A report carrying one image part with a single detected face — the pick
    // routes through the image pipeline (Anonymizer::new(), no rules → no
    // operator), so the group is present and empty.
    let image = PartId::new(IMAGE_PART);
    let report = Report::new().insert_part::<Image>(image.clone(), Vec::new());

    let group = orchestrator
        .select_part(&report, &image, &Scope::default())
        .expect("the image part routes to the image pipeline");
    let selections = group
        .as_any()
        .downcast_ref::<Vec<Selection<Image>>>()
        .expect("image-part selections are Image");
    assert!(
        selections.is_empty(),
        "no entities on the part → no picks, but the group is present",
    );
    Ok(())
}

/// `select_*` returns `None` when nothing routes: an unknown part id, and a
/// report with no body.
#[tokio::test]
async fn select_returns_none_when_nothing_routes() -> Result<()> {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = orchestrator(&registry)?;

    let mut doc = registry.decode(SAMPLE, "docx").await?;
    let report = orchestrator.analyze(&mut doc, &Directives::new()).await?;

    assert!(
        orchestrator
            .select_part(&report, &PartId::new("nope/missing.bin"), &Scope::default())
            .is_none(),
        "an unknown part routes to no pipeline",
    );
    assert!(
        orchestrator.select_body(&Report::new(), &Scope::default()).is_none(),
        "a report with no body has nothing to select",
    );
    Ok(())
}
