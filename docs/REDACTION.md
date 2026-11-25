# Redaction

A conceptual white paper on the redaction layer of the toolkit, the multimodal
PII/PHI detection-and-redaction library. The audience is a privacy engineer
evaluating the architecture; the goal is to describe the model, not the
implementation. Detection produces the entities this paper consumes (see the
Detection paper), and ingestion supplies the streams this paper writes through
(see the Ingestion paper). Extending the catalogue of transformations is a matter of
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
caller, typically the application that embeds the toolkit, states it as an
ordered list of rules wired into the redaction engine before any document is
processed. A rule pairs a selector with an operator: a label (a phone-number
type), a tag the label carries in the catalog (every label tagged `pii`), a
predicate over the entity and the call's scope (its confidence, the request's
purpose or audience), or a fallback that matches everything. For each entity
the first matching rule picks the operator; an entity no rule matches is left
as it is, so a policy that must hide everything ends in a fallback. A rule may
also carry the reason it exists, the policy clause that requires it, which is
recorded on every entity it decides.

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
   keep, remove, cover,              encrypt, pseudonymize
   replace, mask, truncate,          (a deanonymize direction,
   clamp, generalize-date,            key- or vault-held)
   hash, fake, blur, pixelate,
   beep, drop row/column
```

The shipped catalogue is uniform in shape. For text, and for every modality
whose values are text:

- A **passthrough operator** lets the value through verbatim. It records that an
  entity was inspected and consciously preserved. Absence of an operator and an
  explicit decision to keep are semantically different.
- A **removal operator** removes the value. Surrounding content closes around
  the hole, and nothing marks where it was.
- A **cover operator** hides the value in place and keeps its extent: one block
  character per character of text, a box over a PDF's glyphs.
- A **template substitution operator** substitutes a templated token for the
  span, expanding placeholders for the entity's label and matched text and
  defaulting to a bracketed label. The simplest operator; also the most prone to
  leakage when the template is naive.
- **Masking and truncation operators** keep part of the value: a mask rewrites
  it character by character, keeping an optional prefix and suffix and
  preserving its length, so a masked card number still parses as one; a
  truncation keeps only a prefix or a suffix.
- **Generalizing operators** coarsen a value instead of hiding it: a clamp
  collapses numbers beyond a bound into a bucket (an age of 94 becomes `90+`),
  and a date generalizer keeps a date's year or month. A value of the wrong
  shape, text that is not a number or a date, is declined, and by default
  removed.
- **Digest operators** produce a one-way digest, plain or salted SHA-2 or a
  keyed HMAC. The original is absent from the artifact, but identical inputs
  produce identical outputs, preserving equality joins at the cost of
  dictionary-attack exposure.
- A **synthesizing operator** replaces the value with a realistic stand-in of
  the same kind, another name or another address, generated for the entity's
  label and language, so token distributions stay useful.

Image, audio, and tabular content add operators of their own: blur and
pixelation for image regions, a tone over an audio interval, and the dropping
of a whole row or column of a table.

These are the operators a caller binds to labels. Reversible operators, which
recover an original later, form a second category and are the subject of
Section 5.

## 3. The Leak Profile

Every operator must declare a leak profile, a three-way classification of how
much of the original value survives the transformation: its content, or its
extent. Where the value sat (which sentence, cell, or second of audio) is not
counted, since every treatment keeps its surroundings. This is the property a
privacy engineer most needs to reason about, so the toolkit makes it part of the
operator contract rather than an emergent property of configuration. The three
levels are concepts, not configuration knobs:

- **Irrecoverable.** Nothing of the value survives, neither its content nor its
  extent. The removal operator is irrecoverable: the span is gone, and the
  document closes around it, so nothing records its former length. A table
  cell emptied of its value is irrecoverable too: the cell keeps its place, but
  nothing of the value it held. A hash approaches this for the value itself,
  though see below.
- **Partial.** Part of the value survives by design: some of its content, or its
  extent (a cover's length, a box's size, a known silence on a timeline).
  Masking, covering and template substitution are partial. A masked value is
  unreadable but its length survives; a templated replacement reveals that
  _some_ entity of a given label was there.
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
Text         byte range         replace | cover | remove
Tabular      cell coordinate    replace | cover | remove, or drop row/column
Image        bounding box       blur | pixelate | cover
Audio        time interval      beep | cover (silence) | remove
Metadata     field key          replace | cover | remove
```

**Text.** A location is a half-open byte range. The replacement is a
removal, a cover (one `█` per character; in a PDF, the glyphs are deleted and
their space kept under a box of the cover's color), or a replacement string. A
replacement's length may differ from the original, which is why text redaction
batches require coordinate care (Section 6).

**Tabular.** A location is a cell coordinate (row and column, with optional
character offsets and column or sheet names). Tabular reuses the text data and
replacement semantics, so the same operators apply unchanged: a cell value is
rewritten exactly as a text span is. A redaction acts on a cell's contents, not
on the table's schema; whole-column and whole-row operations belong to a
structural operator category distinct from the value-rewriting operators
described here.

**Image.** A location is a bounding box. The replacement value is an image
treatment, and its flavors are not interchangeable: a Gaussian blur preserves a
rough silhouette, mosaic pixelation preserves coarse color statistics, and a
cover, a solid box, preserves nothing but the region's extent. An image can't
lose its pixels, so it has no removal: a region can only be covered in place.
The image operators produce these treatments, and the image codec
applies them.

**Audio.** A location is a time interval. The replacement value silences the
interval (its amplitude is zeroed), beeps over it, or removes it (the buffer
shrinks and downstream timestamps shift). As with image, the audio operators
produce these treatments, and the audio codec applies them.

**Metadata.** A location is a field's key. A field's value is text, so the text
operators apply to it; removing it drops the field.

The split is deliberate. The text operators are written once against a
text-backed notion that text, tabular and metadata satisfy, so a single masking
implementation serves all three. Image and audio replacements have no
meaningful character-level definition, so each has operators of its own.
`Cover` and `Remove` are implemented per modality, for each that can give them:
every modality can be covered, and every one but image can have a value
removed. This keeps the catalogue honest: every operator has a meaningful
definition in every modality it serves.

## 5. Reversibility as a Design Choice

Reversibility is a separate operator category, not a feature flag on individual
operators. A reversible operator extends the ordinary operator contract with a
deanonymize direction: given an entity and the replacement that was written, it
recovers the original data, returning nothing when a particular replacement
cannot be reversed. An operator that holds a key, a vault reference, or a
pseudonym map implements that extended contract; an irreversible operator does
not, and the type system records that fact. Pseudonymization and encryption are the
reversible operators: each substitutes the entity for a surrogate that an
authorized holder of the corresponding secret can map back to the original.
Encryption carries the original in its ciphertext and needs only the key.
Pseudonymization keeps the mapping in a vault, one surrogate per original, or
per coreference cluster so every mention of one person gets the same
surrogate. A deanonymizer applies the reverse direction to a redacted
document.

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
each entity it picks the operator its rules select, reads the covered data
through the stream (see the Ingestion paper), runs the operator to obtain a
replacement value, and records one location-and-replacement pair. Only after
the whole batch is planned does it write back, by handing the batch to the
stream.

The choice of operator can be made, and shown, before anything is written. A
_pick_ records on each entity the operator it will get, the leak profile of
that operator, the rule that chose it, and the rule's reason, as an event on
the entity's audit trail. Applying the batch then records which pick it
applied, recording a new pick first if an edit changed the decision since.
When no edit intervenes, the plan a reviewer was shown is the plan that ran;
when one does, the trail records the pick that was applied in its place, so
the divergence is visible.

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
a redaction span is byte-for-byte identical to the input, once the content the
toolkit strips at decode is gone (see the Ingestion paper). Re-encoding the whole
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
behind the stream boundary. The benefit is that the toolkit can be
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
- **Audit persistence.** Every pick and every applied redaction is recorded
  on its entity's hash-linked audit trail, and the report that carries them
  serializes. Read back normally, it is checked against its digest, kept where
  the report's carrier cannot change it: on the side that wrote it, in a
  write-once store, or signed with a key the deployment holds. An unchecked
  read, named as such, skips that check, for a report that never left the
  side reading it. The toolkit does not persist that record or hold any key;
  a deployment that needs a durable compliance log keeps the report and its
  digest. The digest covers the report's entities and their trails, not its
  record of stripped hidden content.
- **The review interface.** The toolkit records a reviewer's corrections on the
  report between detection and redaction (see the Detection paper), and its
  picks show what redaction will do. Presenting them to a person and
  collecting the decisions is the embedding application's interface.
- **External key management.** A reversible operator consumes a key; it does not
  source it. Vault integration, HSM-backed signing, key rotation, and access
  policy are deployment integrations.
- **Re-identification accounting.** The toolkit does not analyze a corpus for
  k-anonymity, l-diversity, or differential-privacy budgets after redaction. An
  anonymized output is the result of operator-selection discipline, not a
  toolkit guarantee.
