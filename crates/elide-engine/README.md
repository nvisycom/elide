# elide-engine

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Drives detection and redaction across whole documents, part by part.

## Overview

A real document is rarely one modality: a DOCX carries text and embedded
images, and a PDF wraps objects of several kinds. A decoded codec document is a
list of parts, each a stream (content a pipeline reads and redacts in place) or
a blob (the bytes of an embedded document). The codec decodes and re-encodes
them, but it has no recognizers, so it cannot detect or redact. This crate
closes the loop.

The `Orchestrator` holds a format registry and one analyze and anonymize
pipeline per modality. It offers each stream to the pipelines until one matches
its modality, decodes each blob into a document of its own and walks it the
same way, then folds the redacted bytes back up the tree. The caller never
names a modality. A run-wide `Scope` (languages, jurisdictions, labels,
catalog) is set once and shared by every pipeline, and a part whose modality
has no pipeline passes through untouched.

Detection and redaction are two phases, so the findings can be reviewed in
between: suppress a false positive, include a missed one, relabel or relocate
an entity, each recorded on the entity's audit trail.

```rust
# use elide_core::entity::audit::{AuditEvent, PatternRule};
# use elide_core::entity::builtins::EMAIL_ADDRESS;
# use elide_core::entity::Entity;
# use elide_core::modality::text::{Text, TextLocation};
# use elide_core::primitive::Confidence;
# use elide_core::recognition::{Context, Recognizer, Subject};
# use elide_core::Component;
#
# /// Stands in for a real recognizer: finds one fixed address.
# #[derive(Component)]
# struct EmailFinder;
#
# #[async_trait::async_trait]
# impl Recognizer<Text> for EmailFinder {
#     async fn recognize(
#         &self,
#         subject: &Subject<Text>,
#         _ctx: &Context<'_>,
#     ) -> elide_core::Result<Vec<Entity<Text>>> {
#         let text = subject.data().as_str();
#         Ok(text.find("jane@example.com").map(|start| {
#             let location = TextLocation::inline(start..start + 16);
#             let event = AuditEvent::pattern("email", Confidence::MAX, location.clone(), PatternRule::default());
#             Entity::new(EMAIL_ADDRESS.to_ref(), location).with_event(event)
#         }).into_iter().collect())
#     }
# }
#
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> elide_core::Result<()> {
use elide_core::entity::LabelCatalog;
use elide_core::modality::text::Text;
use elide_core::recognition::Scope;
use elide_core::redaction::HiddenContentPolicy;
use elide_detection::Analyzer;
use elide_engine::{Orchestrator, RegistryDocumentExt};
use elide_format::FormatRegistry;
use elide_redaction::operators::Erase;
use elide_redaction::{Anonymizer, Rule};

let registry = FormatRegistry::with_builtin();
let orchestrator = Orchestrator::new()
    .with_registry(registry.clone())
    .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
    .with_modality::<Text>(
        Analyzer::new().with_recognizer(EmailFinder),
        Anonymizer::new().with(Rule::fallback(Erase)),
    );

let text = "Write to jane@example.com";
let mut document = registry
    .document("notes.txt", text, HiddenContentPolicy::default())
    .await?;

// Detect, review the findings, then apply them.
let analyzed = orchestrator.analyze(&mut document).await?;
let part = analyzed.report.part::<Text>(&document.id())?;
assert_eq!(part.entities().len(), 1);
orchestrator.anonymize_with(&mut document, analyzed.report).await?;

let redacted = document.document.encode()?;
assert!(!String::from_utf8_lossy(redacted.as_bytes()).contains("jane@"));
# Ok(())
# }
```

## The two views

Analysis produces two parallel structures, both keyed by each part's path (the
document's name, then each container part's local id), but carrying opposite
things:

- The **report** is *references*: each entity is a location into its part plus
  its audit trail, and nothing more. It carries no live document state, so it
  serializes on its own: a review tool can ship it elsewhere, edit it, and
  rebuild it to apply later. Applying it re-decodes each part from the
  documents, so a rebuilt report redacts exactly as a freshly analyzed one does.
- The **artifacts** are *content*: the enrichment each entity was found in (an
  image's OCR layout, an audio clip's transcript), kept out of the report so it
  stays references only. Persisted across a review gap, the artifacts let a
  later pass re-recognize (running one more recognizer, say) without invoking
  the OCR or speech-to-text models again.

The modality set is open: a downstream crate can define its own modality and
register it. How the two views store several modalities behind one type is an
internal detail; callers work through typed, modality-generic accessors.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE](../../LICENSE)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
