# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Toolkit

- Multimodal detection-and-redaction toolkit for PII/PHI across text, tabular
  data, document metadata, images, and audio
- Every detected entity carries a tamper-evident audit trail: the recognizers
  that found it, how overlapping findings were reconciled, confidence
  adjustments, reviewer decisions, the operator picked to hide it, and the
  redaction applied, verifiable as a hash-linked DAG
- Recognizers, enrichers, and redaction operators share one name-and-version
  identity (`ComponentId`), recorded in the audit trail
- WebAssembly bindings (`elide-wasm`) that run the detect-and-redact pipeline
  in a browser

#### Formats

- Codecs for TXT, JSON, CSV, HTML, XML, DOCX, XLSX, PPTX, PDF, PNG, JPEG, TIFF,
  WAV, and MP3, registered in a `FormatRegistry` resolved by extension, MIME
  type, or content sniffing
- Embedded documents (an image in a DOCX, a file in a PDF) are decoded as
  parts of their container, under their parent's policy, and analyzed and
  redacted through the pipeline registered for their modality; an embedded
  file no codec can read is removed
- Optional PDF page rendering to images via the native PDFium library
  (`pdf-render` feature)

#### Detection

- Pattern-based detection with regex and dictionary recognizers, post-match
  validators (Luhn checksum, SSN format), and built-in dictionaries for
  nationalities, religions, currencies, cryptocurrencies, and languages
- Language detection, and keyword-boost context enhancement over the in-text
  window and out-of-band hints (a column header, a JSON key)
- LLM/VLM-mediated recognizers (text NER and image VLM) over a pluggable rig
  backend
- Speech-to-text and OCR enrichers that make audio and images text-recognizable
- An `Analyzer` per modality that runs enrichers, then recognizers, then the
  layers that calibrate, fuse, arbitrate, and threshold their findings;
  recognizer decorators `resilient`, `chunked`, and `escalation`
- A caller's `Scope` names the entity types to detect (an empty catalog
  detects nothing), the asserted languages and jurisdictions, and free-form
  document tags, purpose, and audience

#### Review

- A `Report` is read and edited through typed part views: `part` and
  `document` return a `Part` (`entities`, `entity`, `suppressed`,
  `unsuppressed`), `part_mut` and `document_mut` a `PartMut` whose edits
  (`include`, `relabel`, `relocate`, `suppress`, `suppress_within`) are
  recorded as `Manual` audit events attributed to its `by` and `because`
- `Orchestrator::pick` records on every entity the operator `anonymize_with`
  will apply, as a `Selection` event, so a review can show the plan before
  anything is applied

#### Redaction

- Redaction operators for text (replace, mask, erase, hash, HMAC, encrypt,
  truncate, clamp, date generalization, pseudonymize, keep), tabular cells
  (drop row, drop column), images (blackbox, blur, pixelate), and audio
  (silence, beep), selected per entity by an `Anonymizer`'s ordered rules
- Coreference-aware pseudonymization with a vault for stable replacements, and
  a `Deanonymizer` for reversible operators
- Every pipeline's anonymizer erases a flagged entity no rule covers, so
  flagged hidden content is never kept for want of a rule

#### Hidden content

- Every codec handles hidden content at decode, under a per-document
  `HiddenContentPolicy` whose metadata and collaboration `Disposition` is
  `KeepOnlyEssential` (the default), `ScanAsContent`, or `FlagEverything`.
  Previews, external references, active content, and editor fingerprints are
  always stripped, every encode checks the output carries none, and a
  document with structure its codec does not recognize is refused
- `FlagEverything` makes every item a format labels an entity whatever the
  catalog declares: each metadata field, each office comment, comment or
  revision author, and tracked deletion, each markup comment, and each HTML
  `<meta>` field, recorded as a `Flagged` audit event naming its kind
- Every stripped item is recorded on the `Report` (`stripped()`) by part,
  item, and count in the format-neutral `HiddenContent` vocabulary
- Metadata is a modality of its own (`Metadata`): an image's EXIF and an
  office document's core, app, and custom properties are keyed text fields
  that each codec labels by its format's schema. Every orchestrator drives
  them with no recognizer: a field whose label the request's catalog declares
  is redacted by that label alone. A caller's `Analyzer<Metadata>` adds
  recognizers that read each value, so a finding inside a free-text field is
  spliced out and the rest kept; a field left blank is removed
- Under `ScanAsContent` an image keeps only the EXIF tags a surfaced field is
  read from, and an EXIF tag goes with its group when another of the group is
  redacted
- PDF: a document encrypted with a user password is refused; catalog, page,
  and trailer entries are kept by allowlist; marked content's alternate text
  is stripped; kept internal links are reduced to their navigation; embedded
  files get neutral names; images are surfaced to the image pipeline, and one
  that cannot be (fax, JBIG2, stencil masks, inline images) is removed rather
  than shipped unscanned
- Office documents surface the text they keep in attributes and outside a
  workbook's cells (authors, sheet, table, and defined names, bookmarks,
  dropdown entries, validation messages, headers and footers, formulas,
  phonetic readings), keep renamed names unique and valid with the links to
  them following, and strip alternative text, tooltips, control labels, XML
  comments, and a workbook's revision log
- HTML reads URLs the way a browser does and strips frames, `<meta>`
  redirects and cookies, conditional comments, remote resource loads and
  pings, `<template>` and `hidden` elements; XML strips `xml-stylesheet` and
  XInclude references and refuses an external DTD

### Crates

- **elide:** Facade re-exporting the toolkit: recognizers, dedup layers,
  redaction operators, and codecs
- **elide-core:** Domain types, traits, and errors
- **elide-detection:** The `Analyzer` and its detection layers
- **elide-pattern:** Pattern and dictionary recognizers, with keyword-boost
  context enhancement
- **elide-ner:** NER traits and language detection
- **elide-lingua:** Lingua-backed language detection
- **elide-llm:** LLM-mediated entity recognizer (text NER and image VLM)
- **elide-backend:** Backend decorators: usage metering, observability sinks,
  rate limiting, routing, and failover
- **elide-redaction:** The operators, the `Anonymizer` and `Deanonymizer`
  that select and apply them, the key vault, and pseudonym generators
- **elide-fake:** Locale-aware fake-data operator
- **elide-engine:** Document orchestration across multi-part containers
- **elide-codec:** Codec traits and the shared extract engine
- **elide-format:** Format registry and the built-in handlers
- **elide-plain:** TXT, JSON, and CSV handlers
- **elide-markup:** HTML and XML handlers, the markup sanitizer, and character
  reference decoding
- **elide-office:** DOCX, XLSX, and PPTX extraction and byte-faithful redaction
- **elide-pdf:** PDF text extraction and born-digital redaction
- **elide-image:** Raster image decode and encode, EXIF read and strip, and
  pixel-region redaction, with OCR enrichment
- **elide-audio:** Audio decode and encode and time-span redaction, with
  speech-to-text enrichment
- **elide-wasm:** WebAssembly bindings for the browser

[Unreleased]: https://github.com/nvisycom/elide/commits/main
