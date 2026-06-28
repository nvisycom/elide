# Detection: A Conceptual Model

## Abstract

This paper describes the conceptual architecture of the recognition layer: the
find phase of a multimodal toolkit for locating sensitive data spans inside
heterogeneous documents. The toolkit is a library, not a service, and this paper
concerns only its first half, the act of finding. The layer is designed around
the observation that no single recognition technique is adequate across all
categories of personally identifiable information (PII), all content encodings,
and all confidence regimes. Multiple recognizer families run concurrently
against decomposed document fragments; their findings are lifted into a shared
coordinate system and reconciled through a deterministic layer pipeline;
caller annotations then shape the surviving candidates into the reconciled set
of entities the toolkit hands to redaction. This is the second paper in a series
of three; ingestion and redaction are treated in their own papers.
Implementation details and the extension surfaces are out of scope here and are
covered by the API reference.

## 1. The Detection Problem

Sensitive data detection is the task of locating spans of content that
constitute personally identifiable information within a document of arbitrary
structure. The problem is harder than it first appears because three axes vary
independently:

- **Category heterogeneity.** PII is not a single class. Email addresses,
  telephone numbers, government identifiers, financial account numbers,
  biometric references, medical history, geolocation traces, and free-form
  references to private third parties all qualify. Each category has its own
  surface form, its own jurisdictional variants, and its own contextual cues.

- **Encoding heterogeneity.** The same logical document can present its contents
  as plain prose, as cells in a spreadsheet, as text regions inside a raster
  image, as transcribed segments of an audio recording, as metadata attached to
  a binary asset, or as fragments embedded in a structured archive. A telephone
  number in a CSV cell is the same datum as a telephone number spoken in an
  interview, but the detection paths leading to them are not.

- **Confidence heterogeneity.** Different recognition techniques produce
  fundamentally different confidence semantics. A regular expression with a
  checksum is either correct or incorrect; the probability mass concentrates at
  the extremes. A neural sequence tagger emits a smooth distribution centered on
  a soft decision boundary. A generative model produces an answer whose
  calibration is opaque. Treating these signals as commensurable requires care.

A detection layer that ignores any of these axes will fail in production.

## 2. Pluralistic Recognition

The central design commitment is pluralism: the toolkit runs a population of
recognizers concurrently and treats each as a hypothesis-generator rather than
an authority. A recognizer is any component that inspects a payload and proposes
the entities it believes are present, in coordinates local to the payload it
saw. Its sole responsibility is that one act of recognition: a recognizer does
not reconcile across recognizers or prune; it reports what it
sees and nothing more. Three families dominate the population.

### 2.1 Rule-Based Recognition

The rule-based recognizer combines pattern matching with deterministic
validators and curated dictionaries. A regular expression establishes a
candidate span; a validator (checksum verification, structural conformance,
range checking) filters out coincidental matches; a dictionary lookup confirms
membership in a known set (country codes, common first names, area codes).
Rule-based recognition has high precision when the rule is well-formed,
near-zero latency, and very low recall outside the patterns it codifies. It
cannot find what it has not been told to look for.

### 2.2 Statistical Recognition

The statistical recognizer draws on sequence-labeling models trained on
annotated corpora. They consume extracted text and emit entity spans with
associated class probabilities. They generalize beyond a closed set of patterns
and capture entities whose surface form is irregular (person names, organization
names, free-form addresses) but their behavior is bounded by the distribution of
their training data. They are noisy near class boundaries and at the edges of
supported languages, and they are sensitive to domain shift. A mock backend
stands in where a real model is not wired up, so the rest of the layer can be
exercised without one.

### 2.3 Generative Recognition

The generative recognizer prompts a large language model, or a vision-language
model for visual payloads, to enumerate entities within a fragment. They are
valuable for categories that resist both rule authorship and supervised
training: open-class identifiers, paraphrased references to sensitive
attributes, contextual mentions ("the patient," "my attorney"). They are also
the path through which caller-supplied candidate regions are adjudicated, since
a generative recognizer can confirm, relocate, or reject a region the caller
only suspected. They are the most expensive recognition path and the least
calibrated. Their utility is highest where the other families are weakest. As
with the statistical family, a mock backend exists so the surrounding machinery
can run without a live model.

### 2.4 No Dominant Strategy

None of these families subsumes the others. Each has a precision-recall profile
shaped by its operating principle. The toolkit does not elect a winner: it runs
the families in parallel and defers the question of which finding to keep to a
later stage that has access to all the evidence at once. The find engine is the
entry point that holds the recognizer population and drives it. Recognizers are
registered with it once; at analysis time it runs them concurrently and collects
their entities into one candidate set.

```
            payload
                |
    +-----------+-----------+
    |           |           |
rule-based  statistical  generative
recognizer  recognizer   recognizer
    |           |           |
    +-----------+-----------+
                |
         candidate set
```

## 3. Chunking and Lifting

Documents are not scanned end-to-end. They are first decomposed into chunks:
paragraphs in prose, cells in tabular content, regions in images, segments in
audio. Chunking bounds the working set for any single recognizer invocation,
preserves locality so recognizer outputs remain interpretable, and exposes
structural context to the recognition layer (a tabular header can inform
interpretation of the cells beneath it).

Each recognizer reports its findings in coordinates local to the chunk it
inspected: a byte offset within a paragraph, a pixel offset within an image
region, a millisecond offset within an audio segment. These chunk-local
coordinates are then lifted back into the source document's coordinate system.
Lifting belongs to the ingestion layer, performed as a decoded source is
streamed chunk by chunk, and it is treated in full by the Ingestion paper; here
it is enough to note that redaction acts on the original document and must
locate every finding inside it, irrespective of which chunk produced the
evidence. A recognizer states a local truth ("a match spanning these bytes of
this chunk"); the lift promotes that truth to source coordinates without the
recognizer ever needing to know the outer geometry.

## 4. Modality Boundaries

Recognition is organized by the _nature of the payload_ presented to a
recognizer, not by the modality of the source document. Text recognizers operate
on textual payloads, regardless of whether the text came from a prose paragraph,
a spreadsheet cell, an OCR pass over an image region, or a transcription pass
over an audio segment. This factoring prevents combinatorial duplication of
recognizer logic across modalities and ensures that improvements to a text
recognizer benefit every upstream extraction path.

The factoring rests on a single idea: any modality whose per-chunk payload is
text can share the same text recognizers. Prose qualifies, and so does tabular
content, where a cell holds text but addresses its entities by row and column.
The text recognizers are written against the text-payload abstraction, not
against a particular modality, so the same rule-based recognizer serves both
prose and tabular cells unchanged, because a cell is text; the only thing that
varies is how a byte match becomes that modality's chunk-local location.

The recognizer population is also open across modalities. Adding a native image
recognizer that emits bounding-box findings, or a native audio recognizer that
emits time-interval findings, does not perturb the existing text recognizers;
the new recognizer is registered alongside the existing ones and contributes
evidence on the same terms. The registry is a fan-out point, not a closed
taxonomy.

## 5. Context Enhancement

Between recognition and reconciliation sits an optional post-recognition pass:
keyword-based confidence boosting, the context-enhancement pass. The premise is
that the words surrounding a candidate carry evidence the recognizer that
produced the candidate may not have weighed. A bare nine-digit number is a weak
social security number on its own; the same number a few words after the phrase
"SSN:" is a much stronger one.

The pass is driven by boost rules, and it is applied by wrapping any text
recognizer so that it behaves as a recognizer in turn. From the find engine's
point of view nothing changed: the wrapper defers to the inner recognizer, then
runs over the produced entities, raising the confidence of those whose
surrounding window matches a rule. Out-of-band context that has no place in the
payload text itself, a column header or a structural key, is surfaced to the
pass through the context hints carried alongside the payload, so a cell value
can be boosted by the column it sits under even though that header is not
adjacent text.

## 6. Per-Call State: Scope and Context

Recognizers do not run in a vacuum; they run inside a call that carries two
distinct kinds of state, separated by who owns them.

The per-call scope holds what the _caller_ asserts about an entire analysis and
is shared, immutable, across every payload of it: the languages and
jurisdictions to honor, document-level classification labels, caller-supplied
include and exclude regions, and a correlation id for tracing. It is built once
and passed by reference into the find engine.

The per-payload context is the working view a recognizer actually sees. It
borrows the scope and adds the state produced while processing one payload: NLP
artifacts (tokens, lemmas, and the like, computed once and read back on demand),
languages detected for this payload, and the context hints described above. The
find engine builds a fresh context per payload, so working state never leaks
between payloads. Recognizers query the call's languages, jurisdictions, and
labels through the context, which folds the caller's assertions together with
what was detected, rather than reaching into the scope directly.

This separation is what lets a jurisdiction-scoped or language-scoped rule
decide whether it applies: the context answers "should a rule restricted to
these countries (or these languages) run for this call?" given both the caller's
assertions and the detected evidence, defaulting to permissive when nothing is
asserted, since applicability cannot be disproved without information.

The per-call scope versus the per-payload working context is a deliberate split:
the caller's assertions are stated once and never mutate, while the state a
recognizer accumulates is rebuilt for each payload and discarded with it.

## 7. Annotations: Caller-Supplied Regions

Caller-supplied annotations are first-class participants in detection, not a
separate workflow. They live on the per-call scope, one per direction.

An include region adds a candidate: the caller believes an entity may lie here,
optionally with a claimed label, name, and confidence. Recognizers that
adjudicate include regions, typically the generative family, fold these into
detection to confirm, relocate, or reject each one; recognizers that do not
adjudicate them leave them alone. An include region amplifies a caller's belief
about sensitivity without dictating the outcome.

An exclude region marks a span that must not appear as a finding. Exclude
regions are applied as a final filter, after reconciliation, removing any
surviving entity whose location overlaps an excluded span regardless of which
recognizer found it. They are how a caller corrects false positives without
retraining a model or rewriting a rule.

## 8. Reconciliation and Conflict Resolution

The parallel evaluation of many recognizers against the same content produces a
redundant candidate set. The same entity is often discovered by multiple
recognizers; spans frequently overlap rather than coincide; class labels
sometimes disagree; and the families do not even share a confidence scale. A
layered pipeline reduces this to a clean, reconciled set.

Each stage is a pure, synchronous transform over the candidate set that returns
the entities it kept and the entities it dropped. The stages are composed onto
the find engine in the order they should run. In their usual order, they are:

- **Calibrate.** Scales each entity's confidence by a per-recognizer multiplier,
  so detectors with different score distributions are made comparable before
  anything is compared. A checksum-backed rule-based hit and a soft neural score
  do not mean the same thing at the same numeric value; calibration is what
  makes the later comparisons meaningful.

- **Reconcile.** Decides what happens to overlapping entities. One stage with
  two axes: a _grouping_ chooses which entities cluster, and a _reconciler_
  chooses what to do with each grouped pair — combine, keep, contest, or
  resolve. Fusion (same-label) and cross-label arbitration are two
  configurations of this one stage, run as two passes:

  - _Same-label._ Co-located findings of the same label are clustered and
    **merged** into one entity over the union of their spans, accumulating every
    contributing detection in the survivor's provenance. The merged confidence
    is pooled, not picked: a rule-based hit and a statistical hit on the same
    span are stronger evidence than either alone (noisy-OR), though the
    conservative default keeps the strongest single score.

  - _Cross-label._ Overlaps between findings of different labels are read
    _structurally_ rather than treated as automatic conflicts. A legitimate
    nesting (a postal code inside an address) keeps both; a subsumed junk match
    (a weak detection inside a much stronger one) is dropped; only a
    near-coincident overlap is a true conflict. A true conflict is either
    **resolved** — the loser dropped, its claim recorded on the winner's
    provenance — or, because "what is this span?" is a meaning question, left
    **contested**: both survive, each flagged, for the human edit step to
    settle. The strict alternative resolves every overlap to one finding per
    span; the permissive alternative keeps every overlap for downstream
    handling.

- **Filter.** Drops entities below a confidence threshold or outside an
  allow-list of labels. This is not noise rejection in the signal-processing
  sense; it is a tunable decision about the precision-recall tradeoff
  appropriate for the deployment.

```
  raw candidate set
         |
         v
+------------------+
|    calibrate     |  rescale per-recognizer confidence
+------------------+
         |
         v
+------------------+
|    reconcile     |  cluster overlaps, then per pair:
|                  |    merge same-label findings,
|                  |    keep / contest / resolve cross-label
+------------------+
         |
         v
+------------------+
|     filter       |  drop below threshold / outside allow-list
+------------------+
         |
         v
  surviving entities
```

Order matters. Calibration before everything else makes the subsequent
comparisons commensurable. Merging same-label findings before arbitrating
cross-label overlaps keeps concurring same-label evidence from being mistaken
for a conflict. Filtering last means a low-confidence finding never displaces a
high-confidence one on its way out. The pipeline is deterministic given the same
inputs. After the stages run, the caller's exclude regions are applied as the
final cull.

## 9. The Reconciled Entity Set

The output of detection is the reconciled set: every entity that survived the
pipeline. Each entity carries:

- a typed label drawn from the toolkit's taxonomy;
- a location in the modality's own coordinate system, already lifted to source
  coordinates when the source was streamed;
- a confidence value derived from the contributing recognizers;
- a provenance trail naming every recognizer that contributed evidence, what
  evidence it contributed, the calibration and fusion that reshaped the score,
  and the before-and-after confidence at each step.

The provenance trail is what makes detection an _evidentiary_ operation rather
than an opaque transformation. The set is serializable, inspectable, and
replayable. A later audit can reconstruct exactly which recognizers fired on
which spans with which confidence, without access to the model weights or rule
internals at the time of the original run. Every decision can be attributed;
every absence can be questioned.

This reconciled set is the contract between detection and the rest of the
toolkit. A redaction pass consumes it and acts on the source document; it does
not re-run detection. What redaction makes of the set is the subject of the
Redaction paper.

## 10. Honest Scope

A few boundaries are worth stating plainly, both about what the toolkit
guarantees and about what it deliberately leaves to whoever embeds it.

The toolkit produces the reconciled set of entities and stops there. What an
embedding engine or service does with that set, routing it to a human reviewer,
letting a reviewer override or correct it at the detection-redaction seam,
persisting an audit record, or expressing policy as a service that projects the
set onto an actionable subset, is out of scope here. The toolkit is a library:
it finds, reconciles, and hands back an attributable set; the orchestration
around that set belongs to its caller. The hooks the toolkit does offer for
steering, the caller's include and exclude regions, are annotations carried
in-band, not a review workflow.

Detection accuracy is bounded by the accuracy of its constituent recognizers.
Rule-based recall is bounded by the rule set: an identifier format the rules do
not encode will not be found by rule. Statistical recall is bounded by training
data, and domain shift between training and deployment degrades performance
silently. Generative recall is bounded by the model: hallucinated entities and
missed entities are both possible, and the confidence the model assigns its own
output is not necessarily informative.

Image and audio are honest gaps. Today, visual and acoustic payloads are reached
only through the generative family, a vision-language model prompted on a
region. Optical-character and speech-to-text extraction, which lets text
recognizers run over regions and segments drawn from images and audio, is
supplied by a backend bound to the recognizer; the toolkit defines the seam and
leaves the backend to the deployment.

The toolkit does not promise zero leakage. It promises that detection is
pluralistic, that evidence is calibrated and reconciled deterministically, that
caller annotations participate as first-class signals, and that the resulting
entity set is attributable. These are engineering invariants, not statistical
guarantees. A deployment that requires statistical guarantees must measure them
on its own data and tune its recognizer population, calibration, thresholds, and
annotations accordingly.

The architecture is open at every layer where recall can fail: recognizers can
be added, rules extended, models replaced, enhancers and layers reconfigured.
The closed surfaces are the ones that must be stable: the chunking and lifting
model, the reconciliation pipeline, and the shape of the entity. Stability in the
contracts is what allows pluralism in the recognizers.

## 11. Summary

Detection is the concurrent evaluation of many fallible recognizers against a
chunked decomposition of a document, followed by an optional context-boost pass,
followed by deterministic reconciliation of the findings through a
calibrate-reconcile-filter pipeline. Caller scope and annotations participate
throughout. The product is an attributable set of entities, lifted into source
coordinates, that the toolkit hands to redaction. The model is conservative
about what it guarantees and generous about what it allows to be extended.
