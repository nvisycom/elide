# Redaction

A conceptual white paper on the redaction layer of the toolkit, the multimodal
PII/PHI detection-and-redaction system separated out of the runtime. The
audience is a privacy engineer evaluating the architecture; the goal is to
describe the model, not the implementation. It is one of a series of three
papers: detection produces the entities this paper consumes (see the Detection
paper), and ingestion supplies the codec handle this paper writes through (see
the Ingestion paper). Extending the catalogue of transformations is a matter of
the operator contract, documented in the API reference.

## Abstract

The toolkit splits its work into two phases. The _analyze_ phase asks where the
sensitive information is and produces a set of typed entities. The _hide_ phase
asks what should happen to each entity and rewrites the document so that, when
re-serialized, it still parses as the format it came in as. This paper describes
the hide phase: the redaction engine, the family of operator contracts it
dispatches over, and the per-modality replacement primitives those operators
emit. It defines the leak taxonomy that lets a caller reason about how much of
an original value survives a transformation, explains why reversibility is a
separate operator category rather than a flag, and is explicit about where the
toolkit provides a primitive versus where it leaves a concern to the engine that
embeds it.

## 1. The Redaction Problem

Detection answers _where is the sensitive information_. Redaction answers the
harder question: _what should happen to it_. The two are routinely conflated,
and the conflation hides a design problem. Once a span has been identified as
personal data, the toolkit must transform it, and every possible transformation
costs something.

Naive removal, deleting the span outright, destroys downstream utility.
Analytics that counted records, machine-learning workflows trained on context
windows, and human reviewers that relied on document structure all degrade when
arbitrary bytes vanish. Naive replacement, substituting a fixed token like
`[REDACTED]`, preserves shape but leaks structure. If every occurrence of one
individual becomes the same token across a corpus, the redaction itself is a
join key: the identifier the operation was supposed to eliminate.

Different deployments demand different trade-offs. A public data release wants
strong anonymization, biased toward irreversibility. An internal audit pipeline
wants reversible pseudonymization so authorized investigators can later recover
the original value. A financial dataset wants format-preserving masking so
downstream validators continue to pass. A machine-learning corpus wants
synthetic but plausible replacements so token distributions remain useful. A
single redaction operation cannot serve all four. The toolkit therefore treats
redaction as a _family of operators_. Each operator is a small contract: given
an entity and the bytes it covers, produce a replacement and a declaration of
how much of the original that replacement leaks.

The toolkit does not own the policy that maps an entity to an operator. The
caller, typically the engine that embeds the toolkit, selects operators by
wiring them into the redaction engine before any document is processed. The
binding is a table from label to operator: a label such as a phone-number type
is bound to a concrete operator, and a single fallback operator covers any label
not otherwise bound. The hide phase consults that table once per entity.

## 2. Operator Taxonomy

An operator is a small, pure contract over a modality. Given an entity and the
modality data it covers, an operator produces a replacement value, and it
declares a leak profile describing what survives. The contract computes a
replacement without mutating the document. The mutation happens later, when the
planned batch is written back.

```
                         operator
                            |
            +---------------+----------------+
            |                                |
         operator                  reversible operator
            |                                |
 +----+-----+------+------+            (deanonymize
 |    |     |      |      |             direction,
keep mask  hash  replace remove         key-held)
```

The shipped catalogue is uniform in shape, and every catalogue entry is a
text-modality operator:

- A **passthrough operator** lets the value through verbatim. It records that an
  entity was inspected and consciously preserved. Absence of an operator and an
  explicit decision to keep are semantically different.
- A **template substitution operator** substitutes a templated token for the
  span, expanding placeholders for the entity's label and matched text and
  defaulting to a bracketed label. The simplest operator; also the most prone to
  leakage when the template is naive.
- A **masking operator** rewrites the span character by character, preserving
  length. It keeps an optional prefix and suffix and replaces the interior with
  a mask character. A masked card number stays the same length and still parses
  as a card number, but its information content is gone.
- A **one-way hash operator** produces a digest, optionally salted, rendered as
  lowercase hex. The original is absent from the artifact, but identical inputs
  produce identical outputs, preserving equality joins at the cost of
  dictionary-attack exposure.
- A **removal operator** removes the span. The replacement disappears and
  surrounding bytes close around the hole. The strongest text operator and the
  most format-disruptive; some codecs cannot honor it without producing invalid
  output.

These are the operators a caller binds to labels. Reversible operators, which
recover an original later, form a second category and are the subject of
Section 5.

## 3. The Leak Profile

Every operator must declare a leak profile, a three-way classification of how
much of the original value or its shape survives the transformation. This is the
property a privacy engineer most needs to reason about, so the toolkit makes it
part of the operator contract rather than an emergent property of configuration.
The three levels are concepts, not configuration knobs:

- **Irrecoverable.** No trace of the original value or its shape remains. The
  removal operator is irrecoverable: the span is gone, and the surrounding bytes
  carry no record of its former length or position once the document closes
  around it. A hash approaches this for the value itself, though see below.
- **Partial.** The original value is gone, but observable shape leaks: position,
  length, a bounding box, cell coordinates, or a known silence on a timeline.
  Masking and template substitution are partial. A masked value is unreadable
  but its length and offset survive; a templated replacement reveals that _some_
  entity of a given label was there.
- **Recoverable.** The original is recoverable from the output given the right
  metadata: an encryption key, a token vault, a pseudonym map, or, in the case
  of an unsalted hash over a small plaintext space, a candidate list to brute
  force against. A passthrough is trivially recoverable because it changes
  nothing. A hash is classified recoverable rather than irrecoverable precisely
  because a small or guessable input domain makes the digest reversible by
  enumeration.

The taxonomy maps onto the familiar privacy vocabulary without adopting its
ambiguity. Suppression and strong anonymization correspond to the irrecoverable
level. Format-preserving masking and tokenized replacement correspond to the
partial level. Reversible pseudonymization and any passthrough correspond to the
recoverable level. Because the profile is declared per operator, a caller can
refuse to deploy any operator weaker than a chosen threshold without inspecting
each operator's internals.

## 4. Per-Modality Replacement Semantics

The operator catalogue is uniform, but the concrete meaning of "replace this
span" depends on the modality. Each modality carries three notions: the data an
operator reads, the location that coordinates a span, and the replacement value
written back.

```
Modality     Location           Replacement primitive
--------     --------           ---------------------
Text         byte range         substitute string, or remove
Tabular      cell coordinate    substitute string, or remove
Image        bounding box       blur | pixelate | block | remove
Audio        time interval      silence | remove
```

**Text.** A location is a half-open byte range. The replacement is either a
substituted string or a removal. Substitution length may differ from the
original, which is why text redaction batches require coordinate care (Section
6).

**Tabular.** A location is a cell coordinate (row and column, with optional
character offsets and column or sheet names). Tabular reuses the text data and
replacement semantics, so the same operators apply unchanged: a cell value is
rewritten exactly as a text span is. A redaction acts on a cell's contents, not
on the table's schema; whole-column and whole-row operations belong to a
structural operator category distinct from the value-rewriting operators
described here.

**Image.** A location is a bounding box. The replacement value is an image
treatment, and its flavors are not interchangeable: a Gaussian blur preserves a
rough silhouette, mosaic pixelation preserves coarse color statistics, a solid
block preserves nothing, and removal excises the region. These treatments are
produced and applied by the image codec handler, not by a text operator.

**Audio.** A location is a time interval. The replacement value silences the
interval (its amplitude is zeroed) or removes it (the buffer shrinks and
downstream timestamps shift). As with image, these are handled by the audio
codec handler rather than a catalogue operator.

The split is deliberate. The catalogue operators are text-modality operators,
written once against a text-backed notion that both text and tabular satisfy, so
a single masking implementation serves both. Image and audio replacements have
no meaningful character-level definition, so they are expressed as replacement
values the codec handler interprets, not as operators a caller binds to a label.
This keeps the catalogue small: every catalogue operator has a meaningful
definition in every modality it claims to serve.

## 5. Reversibility as a Design Choice

Reversibility is a separate operator category, not a feature flag on individual
operators. A reversible operator extends the ordinary operator contract with a
deanonymize direction: given an entity and the replacement that was written, it
recovers the original data, returning nothing when a particular replacement
cannot be reversed. An operator that holds a key, a vault reference, or a
pseudonym map implements that extended contract; an irreversible operator does
not, and the type system records that fact. Pseudonymization, encryption, and
tokenization are the reversible operators: each substitutes the entity for a
surrogate that an authorized holder of the corresponding secret can map back to
the original.

The distinction matters because reversible operators carry obligations the
others do not. They require key management, rotation, and access controls. They
carry liability irreversible operators lack: a stolen key is a stolen dataset.
They are appropriate for internal pipelines and inappropriate for public
releases. Folding them under a single "redact" verb would hide exactly the
property the privacy engineer most needs to reason about, which is why the
toolkit exposes reversibility at a trait boundary so the choice is deliberate at
the point an operator is bound to a label.

The forward transform and its inverse share secret state: a key, a vault, a
pseudonym map. That state is the operator's responsibility, not the toolkit's.
The toolkit defines the seam at which a reversible operator attaches and the
deanonymize direction it must provide; it does not impose a key-management
policy, because key custody is a security decision that belongs to the
deployment rather than the library. A reversible operator's replacement is
classified `Recoverable` by its leak profile, so the two declarations agree: the
value can come back, and the toolkit says so in both the type and the profile.

## 6. The Redaction Batch

A document typically produces many entities. The redaction engine does not
rewrite the document entity by entity. It first _plans_ a batch of rewrites: for
each entity it resolves the bound operator (falling back if necessary), reads
the covered bytes through the codec handle (see the Ingestion paper), runs the
operator to obtain a replacement value, and records one location-and-replacement
pair. Only after the whole batch is planned does it write back, by handing the
batch to the codec handle's writer.

Planning before writing exists because writing is destructive and ordering
matters. For codecs whose write step shifts byte offsets, text, tabular, and any
stream where a replacement can change buffer length, the batch is applied right
to left. A redaction near the end of the buffer is written first, so an earlier
insertion or deletion cannot invalidate the coordinates of a later one.

```
   buffer: [....a.....b.......c....]
                |     |       |
            entity1 entity2 entity3

   apply order: entity3, entity2, entity1
```

The batch is iterable in both directions, so the writer walks it in reverse when
offsets are fragile. For codecs whose write step does not shift offsets, image
bounding boxes are independent and tabular cell coordinates are stable, the
batch order is immaterial. Right-to-left application is a property the codec
writer enforces for the modalities that need it, not a global invariant imposed
on every modality.

## 7. Format-Preserving Output

After redaction, the modified artifact re-serializes to its original container
format. A redacted CSV is still a valid CSV; a redacted PDF is still a valid
PDF; a redacted WAV is still a valid WAV. Two properties follow. First,
non-redacted bytes are preserved: when the codec supports it, every byte outside
a redaction span is byte-for-byte identical to the input. Re-encoding the whole
artifact would be simpler but would subtly alter content the caller never
intended to touch: floating-point columns re-rounded, image compression
artifacts shifted, audio samples re-quantized. The toolkit treats avoidable
re-encoding as a defect. Second, the format contract is preserved: downstream
consumers that expect a specific column ordering or a PDF whose embedded fonts
match an external reference continue to work.

The cost is that the hide phase threads its writes through codec-specific
writers that understand how to splice modifications into the original stream
(see the Ingestion paper). The redaction engine itself is modality-generic; the
knowledge of how to apply a batch to a particular container lives in the codec
handler behind the writer boundary. The benefit is that the toolkit can be
inserted into a pipeline without forcing every consumer to tolerate a
re-serialized variant of its input.

## 8. Honest Scope

The toolkit provides redaction primitives, the operators, the leak taxonomy, the
batch, and the format-preserving writers, and deliberately leaves several
adjacent concerns to the engine that embeds it.

- **Policy.** The toolkit does not decide which operator applies to which entity
  per deployment. The caller wires operators to labels, with a fallback, before
  any document is processed. Selecting operators by deployment, by jurisdiction,
  or by data-release target is the embedding engine's job.
- **Audit persistence.** The hide phase plans and applies redactions; it does
  not maintain a durable, append-only audit log of every decision and execution.
  A deployment that needs a compliance record builds it around the planned
  batch.
- **Human override at the seam.** The toolkit consumes the entity set produced
  by detection (see the Detection paper) and acts on it. Presenting detected
  entities to a reviewer and collecting accept, reject, or re-operator decisions
  before the destructive write is an interface concern owned by the embedding
  service.
- **External key management.** A reversible operator consumes a key; it does not
  source it. Vault integration, HSM-backed signing, key rotation, and access
  policy are deployment integrations.
- **Re-identification accounting.** The toolkit does not analyze a corpus for
  k-anonymity, l-diversity, or differential-privacy budgets after redaction. An
  anonymized output is the result of operator-selection discipline, not a
  toolkit guarantee.
