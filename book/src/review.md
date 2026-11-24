# Reviewing before redacting

Automatic detection is not perfect, and many redaction workflows have a
person check its work: drop a false positive, add something it missed. The
orchestrator splits a redaction in two for this. `analyze` finds entities
and returns a `Report`; `anonymize_with` applies a report. Between them, a
reviewer's decisions arrive as a `Review`, which `review` applies.

```rust,no_run
use elide::entity::audit::Attribution;
use elide::modality::text::TextLocation;
use elide::prelude::*;
use elide::{Protect, RequireActor, RequireReason, Review, ReviewPolicy};

# async fn review(
#     orchestrator: Orchestrator,
#     mut document: Document,
# ) -> std::result::Result<(), Box<dyn std::error::Error>> {
let orchestrator = orchestrator.with_review_policy(
    // Nobody may spare a government ID, and whoever spares anything else
    // must say who they are and why.
    Protect::labels([builtins::GOVERNMENT_ID.to_ref()])
        .and(RequireActor)
        .and(RequireReason),
);
let analyzed = orchestrator.analyze(&mut document).await?;
let mut report = analyzed.report;

// Show the reviewer what was found, and collect their decisions.
let mut review = Review::new(&report);
for (part, _) in report.part_ids() {
    let Ok(view) = report.part::<Text>(part) else { continue };
    for entity in view.entities() {
        println!("{part}: {} at {:?}", entity.label().as_str(), entity.location());
        # let reviewer_says_not_sensitive = *entity.label() == builtins::ADDRESS.to_ref();
        if reviewer_says_not_sensitive {
            review
                .part::<Text>(part)?
                .by("reviewer-7")
                .because(Attribution::freeform("public company address"))
                .suppress(entity.id());
        }
    }
}

// Clear a whole region of the document's own text, such as a public
// letterhead, in one edit.
review
    .part::<Text>(&document.id())?
    .by("reviewer-7")
    .because(Attribution::freeform("letterhead"))
    .suppress_within(TextLocation::inline(0..120));

// Apply it: every edit, if the policy accepts them all, or none.
orchestrator.review(&mut report, &review)?;

// The applied report carries each entity's trail, every decision included:
// keep it. Its digest covers the entities, not the stripped section.
let applied = orchestrator.anonymize_with(&mut document, report).await?;
std::fs::write("contract.report.json", serde_json::to_vec(&applied)?)?;
println!("report digest: {}", applied.digest());
# Ok(())
# }
```

A report keeps each part's entities apart, keyed by `PartId`. `part` reads
one as a modality; a document's own content is keyed by `document.id()`, and
its container parts beneath it. A review edits parts the same way: `part` on
a review takes the edits for one part, which can:

- **Include** an entity the analysis missed: `include` adds one of a label at
  a location. The location can be a span, or a mark naming text by where it
  sits: raw bytes in a part (`TextLocation::marked_source`, for office,
  markup, JSON and plain text) or a region of a rendered PDF page
  (`marked_rendered`). Source references address a part as decoded, with its
  hidden content already handled, so take them from the report rather than
  from the original file.
- **Suppress** an entity: `suppress` keeps it in the report, marked
  suppressed, so it is not redacted.
- **Suppress a region**: `suppress_within` suppresses every entity the region
  contains. A finding reaching outside the region is left alone, so nothing
  outside it is spared.
- **Amend** an entity: `relabel` gives it another label and `relocate` moves
  it.

`review` applies the edits in order. Each is recorded on the entity it
touches as a manual audit event, with who made it (`by`), why (`because`),
and what it set: the label an include or relabel gave, the location an
include or relocation moved the entity to, the region a suppression covered.
A human decision is never mistaken for an automatic one, and the trail says
what the entity became. Afterwards, `suppressed` and `unsuppressed` on a part
list which entities redaction leaves alone and which it considers.

When the report is applied, each mark resolves to the span of text it covers,
keeping the reviewer's anchor, and is redacted like any detection. A mark
must cover one run of text: pieces with anything but whitespace between them
(two boxes on a page, a box over a table column) would take that text too, so
mark each piece on its own. A mark the document cannot place, like any
location a part cannot read or write, fails the apply rather than leaving
content unredacted.

## What a review may do

An include only adds to what is redacted, and so does a relocation to a
region containing the old one. Every other edit can leave content unredacted
that would have been hidden: a suppression, a relabel (another label may get
a weaker operator), a relocation that gives up any of the old location. The
orchestrator's `ReviewPolicy` governs those. It is shown each edit as a
`ProposedEdit`: the decision it records, the entities whose redaction it
narrows, and who made it and why. Each standard policy checks one thing:
`Protect` keeps labels from being narrowed, `RequireActor` wants every edit
attributed, and `RequireReason` wants every narrowing edit explained. `and`
combines them, and a deployment's own rules (only some reviewers may narrow,
say) are a `ReviewPolicy` too, or a closure. The default accepts every edit.

Each edit is checked as the edits before it left the part, so an entity
moved into a region is protected there. A review the policy refuses is
refused whole, with an error of kind `Refused`, and the report is left as it
was.

The policy checks what a review claims, not who sent it: `by` is a name the
review carries. Authenticate reviewers before their reviews reach `review`.

A review is bound to the report it was made against: `Review::new` records
the report's digest, and `review` refuses a review made against another
report, or an earlier version of this one. Applying a review changes the
digest, so the same review cannot be applied twice, and an older one cannot
replace a newer.

## Showing the reviewer

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
`anonymize_with` applies. When no review intervenes, the plan shown is the plan
that runs; after one, pick again to update it. An unchanged pick is not
recorded twice.

Applying records a `Redaction` event naming, by its hash, the `Selection` it
applied, and the location the operator wrote. If an edit changed the decision
since the pick, the new `Selection` is recorded first and the `Redaction`
names that one. `audit.applied_selection()` reads what a redaction ran, and
`verify` rejects a `Redaction` that names no earlier `Selection`.

## Across processes

A `Report` and a `Review` both serialize, so detection, review, and redaction
can run in different processes, or days apart with a review queue between
them. Neither can protect itself against whoever carries it. Each entity's
trail is hash-linked, but the hashes are not keyed: whoever rewrites an
entity can recompute its trail, a trail cut back to an earlier event is
intact in its own right, a label or location changed beside its trail leaves
the trail as it was, and dropping an entity or an edit leaves everything else
intact.

So each is sealed when it is written and reads back only against its seal.
A `Seal` is plain data, made with one of:

- `Seal::of(&report)`, an unkeyed digest. Anyone can compute one, so keep it
  where the carrier cannot write: a signed log, a write-once bucket, a
  timestamping service.
- `Seal::keyed(&report, &key)`, a digest keyed with a `SealKey` the carrier
  does not hold. Only a holder of the key can compute one, so it can be stored
  beside what it covers.

`Report::restorer()` (or `orchestrator.restorer()`) reads either back
with `restore(&seal, ..)`, and refuses it if it does not match; a keyed seal
needs the key, set with `with_seal_key` on the restorer or the
orchestrator. A report's seal covers every entity's fields and trail, by part
and entity id, so the order a report was built or serialized in does not
matter. A review's covers the report it was made against and every edit, who
made it, and why. `restore_unsealed` skips the check, for a value only shown, or
one that never left the side that reads it.

The analysis also returns `artifacts`: the OCR layouts and transcripts it
produced. Pass them to `re_analyze` to detect again, under a new scope or
with another recognizer, without running OCR or speech-to-text a second
time. An altered layout would change what detection reads, so an artifact set
is sealed and read back the same way, its seal covering every artifact as it
was serialized.
