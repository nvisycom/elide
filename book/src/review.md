# Reviewing before redacting

Automatic detection is not perfect, and many redaction workflows have a
person check its work: drop a false positive, add something it missed. The
orchestrator splits a redaction in two for this. `analyze` finds entities
and returns a `Report`; `anonymize_with` applies a report.

```rust,no_run
use elide::entity::audit::Attribution;
use elide::modality::text::TextLocation;
use elide::prelude::*;

# async fn review(orchestrator: Orchestrator, mut document: Document) -> Result<()> {
let analyzed = orchestrator.analyze(&mut document).await?;
let mut report = analyzed.report;

// Show the reviewer what was found in the text parts.
let mut rejected = Vec::new();
for (part, _) in report.part_ids() {
    let Ok(view) = report.part::<Text>(part) else { continue };
    for entity in view.entities() {
        println!("{part}: {} at {:?}", entity.label.as_str(), entity.location);
        # let reviewer_says_not_sensitive = entity.label == builtins::ADDRESS.to_ref();
        if reviewer_says_not_sensitive {
            rejected.push((part.clone(), entity.id));
        }
    }
}

// Keep what the reviewer rejected from being redacted, and say who and why.
for (part, id) in rejected {
    report
        .part_mut::<Text>(&part)?
        .by("reviewer-7")
        .because(Attribution::freeform("public company address"))
        .suppress(id);
}

// Clear a whole region of the document's own text, such as a public
// letterhead, in one call.
report
    .part_mut::<Text>(&document.id())?
    .by("reviewer-7")
    .suppress_within(&TextLocation::inline(0..120));

orchestrator.anonymize_with(&mut document, report).await?;
# Ok(())
# }
```

A report keeps each part's entities apart, keyed by `PartId`. `part` reads
one as a modality and `part_mut` edits it; a document's own content is keyed
by `document.id()`, and its container parts beneath it. Through `part_mut`, a
reviewer can:

- **Suppress** an entity: `suppress` keeps it in the report, marked
  suppressed, so it is not redacted.
- **Suppress a region**: `suppress_within` suppresses every entity the region
  contains, and returns which ones. A finding reaching outside the region is
  left alone, so nothing outside it is spared.
- **Include** one the analysis missed: `include` adds an entity of a label at
  a location.
- **Amend** an entity: `relabel` gives it another label and `relocate` moves
  it.

Every edit is recorded on the entity's audit trail as a manual event, with
who made it (`by`) and why (`because`), so a human decision is never mistaken
for an automatic one. Afterwards, `suppressed` and `unsuppressed` on a part
list which entities redaction leaves alone and which it considers.

To show the reviewer what redaction will do, not only what was found, call
`orchestrator.pick(&mut report)`. Each entity records the operator it will get,
how much that operator's output leaks, the rule that chose it, and the policy's
rationale, as a `Selection` event, picked by the same anonymizers and scope
`anonymize_with` applies, so the plan shown is the plan that runs. Pick again
after an edit to update it; an unchanged pick is not recorded twice.

Applying records a `Redaction` event naming, by its hash, the `Selection` it
applied. If an edit changed the decision since the pick, the new `Selection` is
recorded first and the `Redaction` names that one. `audit.applied_selection()`
reads what a redaction ran, and `verify` rejects a `Redaction` that names no
earlier `Selection`.

## Across processes

A `Report` serializes, so detection and redaction can run in different
processes, or days apart with a review queue between them.
`Report::deserializer()` reads one back.

The analysis also returns `artifacts`: the OCR layouts and transcripts it
produced. Pass them to `re_analyze` to detect again, under a new scope or
with another recognizer, without running OCR or speech-to-text a second
time.
