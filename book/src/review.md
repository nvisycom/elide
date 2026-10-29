# Reviewing before redacting

Automatic detection is not perfect, and many redaction workflows have a
person check its work: drop a false positive, add something it missed. The
orchestrator splits a redaction in two for this. `analyze` finds entities
and returns a `Report`; `anonymize_with` applies a report.

```rust,no_run
use elide::entity::audit::Attribution;
use elide::prelude::*;

# async fn review(orchestrator: Orchestrator, mut document: Document) -> Result<()> {
let analyzed = orchestrator.analyze(&mut document, &Directives::new()).await?;
let mut report = analyzed.report;

// Show the reviewer what was found in the text parts.
let mut flagged = Vec::new();
for (part, _) in report.part_ids() {
    let Some(entities) = report.part_entities::<Text>(part) else { continue };
    for entity in entities {
        println!("{part}: {} at {:?}", entity.label.as_str(), entity.location);
        # let reviewer_says_not_sensitive = entity.label == builtins::ADDRESS.to_ref();
        if reviewer_says_not_sensitive {
            flagged.push((part.clone(), entity.id));
        }
    }
}

// Keep what the reviewer rejected from being redacted, and say why.
for (part, id) in flagged {
    report.suppress_part::<Text>(
        &part,
        id,
        Some(Attribution::freeform("public company address").into()),
        Some("reviewer-7".to_owned()),
    );
}

orchestrator.anonymize_with(&mut document, report).await?;
# Ok(())
# }
```

A report keeps each part's entities apart, keyed by `PartId`. It can:

- **Suppress** an entity: `suppress_part` keeps it in the report, marked
  suppressed, so it is not redacted.
- **Include** one the analysis missed: `include_part`, or `include_custom`
  for a single-part document, with an entity built by `Entity::custom`.
- **Edit** entities in place: `part_entities_mut`, `entity_mut`.

Every edit is recorded on the entity's audit trail as a manual event, with
who made it and why, so a human decision is never mistaken for an automatic
one.

## Across processes

A `Report` serializes, so detection and redaction can run in different
processes, or days apart with a review queue between them.
`Report::deserializer()` reads one back.

The analysis also returns `artifacts`: the OCR layouts and transcripts it
produced. Pass them to `re_analyze` to detect again, under a new scope or
with another recognizer, without running OCR or speech-to-text a second
time.
