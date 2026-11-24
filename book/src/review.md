# Reviewing before redacting

Automatic detection is not perfect, and many redaction workflows have a
person check its work: drop a false positive, add something it missed. The
orchestrator splits a redaction in two for this. `analyze` finds entities
and returns a `Report`; `anonymize_with` applies a report.

```rust,no_run
use elide::entity::audit::Attribution;
use elide::modality::text::TextLocation;
use elide::prelude::*;

# async fn review(
#     orchestrator: Orchestrator,
#     mut document: Document,
# ) -> std::result::Result<(), Box<dyn std::error::Error>> {
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

// The applied report carries each entity's trail, redaction included: keep
// it. Its digest covers the entities, not the stripped section.
let applied = orchestrator.anonymize_with(&mut document, report).await?;
std::fs::write("contract.report.json", serde_json::to_vec(&applied)?)?;
println!("report digest: {}", applied.digest());
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
  a location. The location can be a span, or a mark naming text by where it
  sits: raw bytes in a part (`TextLocation::marked_source`, for office,
  markup, JSON and plain text) or a region of a rendered PDF page
  (`marked_rendered`). Source references address a part as decoded, with its
  hidden content already handled, so take them from the report rather than
  from the original file.
- **Amend** an entity: `relabel` gives it another label and `relocate` moves
  it.

When the report is applied, each mark resolves to the span of text it covers,
keeping the reviewer's anchor, and is redacted like any detection. A mark
must cover one run of text: pieces with anything but whitespace between them
(two boxes on a page, a box over a table column) would take that text too, so
mark each piece on its own. A mark the document cannot place, like any
location a part cannot read or write, fails the apply rather than leaving
content unredacted.

Every edit is recorded on the entity's audit trail as a manual event, with
who made it (`by`) and why (`because`), so a human decision is never mistaken
for an automatic one. Afterwards, `suppressed` and `unsuppressed` on a part
list which entities redaction leaves alone and which it considers.

A report keeps no matched values, so to show the reviewer what each entity
covers, read it from the documents before they are redacted:
`orchestrator.read::<Text>(&mut documents, &locations)` takes `(PartId,
location)` pairs, in any part including those inside embedded files, and
returns the data at each. A location fails on its own, so one the documents
cannot read costs only its own result, but an embedded file that fails to
decode on the way fails the whole read.

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
`Report::deserializer()` reads one back, and refuses one whose audit trails do
not verify: an event altered in transit, or cut from below a trail's last
event, fails the read.

That check has limits. The trail hashes are not keyed, so whoever rewrites a
whole entity can recompute its trail; a trail cut back to an earlier event is
intact in its own right; and trails are per entity, so dropping an entity
outright leaves every other trail intact. To catch those, record
`report.digest()` after analysis (and again after a trusted review, since
edits change it) somewhere the report's carrier cannot write, such as a
signed log, a write-once bucket, or a timestamping service, and compare it
with the digest of the report you are about to apply. The digest covers every
entity's fields and trail.

The analysis also returns `artifacts`: the OCR layouts and transcripts it
produced. Pass them to `re_analyze` to detect again, under a new scope or
with another recognizer, without running OCR or speech-to-text a second
time.
