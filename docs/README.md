# elide: A Multimodal PII Detection and Redaction Toolkit

## Abstract

This document series describes the conceptual architecture of `elide`, a library
for detecting and redacting personally identifiable information across
heterogeneous data: free text, tabular records, still images, and recorded
audio. The toolkit addresses three problems that a single-modality redactor
cannot. First, sensitive content surfaces through different carriers in each
modality (a span of characters, a cell value, a region of pixels, an interval of
waveform), and any unified treatment must respect those carriers rather than
reduce them to a common substrate. Second, no single detection technique is
sufficient: deterministic patterns recover structured identifiers with high
precision, statistical models recover unstructured entities with useful recall,
and generative models recover context-dependent mentions that neither of the
prior two can frame. The toolkit therefore admits a pluralistic detection layer
in which multiple recognizers contribute to a single annotation set. Third,
redaction must be auditable and, in specific cases, reversible by an authorized
party, so the toolkit treats the rewrite operation itself as a first-class
object with a declared kind (suppression, anonymization, pseudonymization) and a
recorded provenance.

`elide` is a library, not a service. It detects and redacts; it does not host a
server, persist an audit trail, schedule work, or carry a policy surface. Those
concerns belong to the engine that embeds the toolkit. The boundary is
deliberate, and the documents below describe what falls on the toolkit side of
it.

## Reader's guide

The remaining documents each take one slice of the toolkit and develop it in
isolation. They are independent and may be read in any order, though the order
below reflects the flow of a document through the toolkit.

| Document                  | Subject                                                                                                                                                                                                                                                                                                  |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Ingestion](INGESTION.md) | How raw bytes become a typed, addressable handle on which the rest of the toolkit operates: format resolution, the decoder-and-handle split, streaming by chunk, the decode-redact-encode loop, and the uniform promotion of chunk-local coordinates to source coordinates.                              |
| [Detection](DETECTION.md) | The composition of rule-based, statistical, and generative recognizers into a single layer that produces a unified set of entity annotations, including the per-call scope, caller-supplied include and exclude regions, and the treatment of overlap, disagreement, and confidence between recognizers. |
| [Redaction](REDACTION.md) | The translation of detected entities into concrete rewrites or removals on the original document, the catalogue of operator kinds, the leak profile that classifies each, the per-modality replacement semantics, and the reversible-operator boundary.                                                  |

## Glossary

The terms below are used throughout the series with the meanings given here.
They are conceptual definitions, not references to any particular interface.

- **Modality**: a class of data carrier with its own internal structure and its
  own notion of location: text, tabular records, still images, and recorded
  audio are the four modalities the toolkit treats.
- **Entity**: a single occurrence of sensitive information within a document,
  located in one modality, of one declared kind (person name, identifier, face
  region, spoken interval, and so on).
- **Location**: the modality-specific coordinate that identifies where an entity
  lives in its host document: a character span, a row and column, a pixel
  region, or a time interval.
- **Chunk**: a unit of a document a handle yields while streaming, in the
  modality's own coordinate system. A recognizer sees a chunk and reports
  findings in chunk-local coordinates.
- **Lifting**: the promotion of a chunk-local location to a source-global one,
  the same operation in every modality, so that everything downstream speaks the
  source's coordinates.
- **Recognizer**: a component that inspects content and proposes entity
  annotations. The toolkit composes several, of different techniques, into one
  detection layer.
- **Redaction**: the act of replacing, removing, or otherwise altering an entity
  in the host document so that it no longer conveys the sensitive information it
  originally carried.
- **Operator**: the transformation that performs one redaction. Each entity is
  assigned an operator, selected by the caller per label, that emits the
  replacement value for that entity.
- **Leak profile**: the classification of how much a redaction leaves
  observable: irrecoverable, partial, or recoverable.
- **Anonymization**: a redaction that severs the link between the redacted
  document and the original entity in a way the toolkit itself cannot reverse.
- **Pseudonymization**: a redaction that substitutes the entity for a token or
  surrogate value, where the substitution can be reversed by a party in
  possession of the appropriate key or mapping.
- **Deanonymization**: the inverse of a pseudonymizing redaction, available only
  where the original redaction was declared reversible and the reverser holds
  the requisite authority.
- **Scope**: the per-call set of controls the caller asserts: languages,
  jurisdictions, document labels, and the include and exclude regions that steer
  recognition.
