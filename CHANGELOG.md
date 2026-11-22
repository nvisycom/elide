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
- Recognizers, enrichers, and redaction operators are each a `Component`
  with one name-and-version identity (`ComponentId`), recorded in the audit
  trail; `#[derive(Component)]` takes it from the crate, fixed attributes, a
  name field, or a wrapped component
- Every backend response names the model that produced it, so an entity
  found by a model is attributed to the model that answered its request,
  through language routing and failover alike
- WebAssembly bindings (`elide-wasm`) that run the detect-and-redact pipeline
  in a browser

#### Formats

- Codecs for TXT, JSON, CSV, HTML, XML, DOCX, XLSX, PPTX, PDF, PNG, JPEG, TIFF,
  WAV, and MP3, registered in a `FormatRegistry` resolved by extension, MIME
  type, or content sniffing
- Embedded documents (an image in a DOCX, a file in a PDF) are decoded as
  parts of their container, under their parent's policy, and analyzed and
  redacted through the pipeline registered for their modality; an embedded
  document nothing was redacted in keeps its original bytes, and an embedded
  file no codec can read is removed
- Optional PDF page rendering via the native PDFium library (`pdf-render`
  feature), which redacts a page from its render: the page is rendered, the
  image pipeline detects on what it shows (OCR) and redacts it, and the page
  is replaced by that render with its redacted embedded images. Scanned pages
  are redacted this way, and so are pages that draw text whose glyphs can't be
  placed, in a font whose widths can't be known or written vertically (without
  the feature, or under `RasterMode::Never`, such a page is refused); `RasterMode::Always` redacts every page this way at
  its configured DPI, never reading the text layer. A page replaced by its
  render that no image pipeline read is refused rather than shipped
  unredacted

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

- A `Report` is read and edited through typed part views, keyed by `PartId`
  (a document's own content by `Document::id`): `part` returns a `Part`
  (`entities`, `entity`, `suppressed`, `unsuppressed`), `part_mut` a `PartMut`
  whose edits (`include`, `relabel`, `relocate`, `suppress`,
  `suppress_within`) are recorded as `Manual` audit events attributed to its
  `by` and `because`
- `Orchestrator::pick` records on every entity the operator `anonymize_with`
  will apply, as a `Selection` event, so a review can show the plan before
  anything is applied
- A reviewer can mark text by where it sits rather than by span: raw bytes in
  a part (DOCX, PPTX, HTML, XML, JSON, TXT) or a region of a rendered
  PDF page. Applying the report resolves each mark to the text it covers
- Applying a report fails closed: a part with entities that no registered
  pipeline can apply, or that names no part of the documents, a mark the
  document cannot place, and a location a stream cannot read or write are
  each an error rather than content left unredacted

#### Redaction

- Redaction operators for text (replace, mask, hash, HMAC, encrypt, truncate,
  clamp, date generalization, pseudonymize), tabular cells (drop row, the
  header row included; drop column), images (blur, pixelate), and audio
  (beep), selected per entity by an `Anonymizer`'s ordered rules;
  `Cover` and `Keep` serve every modality, and `Remove` every one that can
  make a value vanish (an image can't lose its pixels, so it has none):
  `Remove` leaves nothing that marks where a value was, `Cover` hides it in
  place and keeps its extent (one `█` per character in text, a box in an
  image, silence in audio), the box drawn in a color of the rule's choosing,
  black by default, and `Keep` writes nothing, so a kept entity is left
  exactly as it was in every format and is recorded as selected, not redacted
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
- PDF: decode rebuilds the document, copying only what draws its pages, its
  internal links, and its attached files: page content is re-encoded through
  the operator set (hidden layers, marked-content property lists, inline
  images, and undefined operators cut), every resource is copied with the
  keys its kind defines and its values resolved, each value only where its
  kind can hold it (so no string rides in a number's place, and a file path
  wherever it is given is recorded, not copied), a resource no page draws is
  left behind, a visible form field's appearance is drawn into its page (with
  its annotation's blend mode and opacity) so its answer is detected like any
  text, and what the rebuild does not copy is recorded by category; encode
  checks its output and refuses one a rebuild would change. A document
  encrypted with a user password, or whose attached files are encrypted under
  a crypt filter of their own, is refused; attached files get neutral names;
  every image, stencil masks included, is surfaced to the image pipeline
  wherever it is drawn from, and one that cannot be (fax, JBIG2, inline
  images, external data) is removed rather than shipped unscanned; text in a
  tiling pattern, a form or a soft mask's group is detected and deleted like
  the page's, and the kerning beside a deleted glyph is zeroed; a glyph
  reads as its font's `/ToUnicode` gives it, ahead of its encoding, as a
  viewer reads it, and a `/ToUnicode` the reader cannot parse is dropped; text a
  font maps to a presentation form (a Kangxi radical, a ligature, an Arabic
  presentation form, a CJK compatibility ideograph) is read as the text it
  writes, and the output's `/ToUnicode` CMaps map it the same way, so copy
  and search in a viewer give that text; glyphs are placed by their font's
  widths, a Type3 font's scaled by its `/FontMatrix`; a standard 14 font that
  embeds no program (or one named by an alias, such as `Arial`) is written
  with every code's glyph, and, without `/Widths` of its own, every code's
  width from Adobe's metrics for its face, so every viewer selects and places
  its glyphs as they were read; where a simple font's `/ToUnicode` maps no
  code, its text reads from the glyph name its encoding gives it (by the
  Adobe Glyph List), and a code whose text can't be known is refused; a font
  that embeds a Type 1, TrueType, OpenType or CFF program is written with
  what the program gives, when it can be read: every code's width, when the
  font gives no `/Widths`, and every code's glyph name, for a Type1 font whose
  encoding leaves codes to the program's own (a font whose program gives
  neither is redacted from its render, or refused under `RasterMode::Never`); content is charged as
  it is decoded, however many times a stream is drawn; a page whose content
  cannot be read, text drawn before a font is selected, content nested too
  deep, and an encode naming a part the document never surfaced are refused
  rather than passed through. The output is untagged (its structure tree
  carried alternate text), and embedded font programs are copied as stored.
  Metadata and collaboration content are stripped and recorded whatever the
  `HiddenContentPolicy` asks
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
- **elide-macros:** Derives for `Component`, `Request`, and
  `Response`, re-exported by `elide-core`
- **elide-detection:** The `Analyzer` and its detection layers
- **elide-pattern:** Pattern and dictionary recognizers, with keyword-boost
  context enhancement
- **elide-ner:** The NER recognizer, its backend contract, and label and
  score decorators
- **elide-lingua:** Lingua-backed language detection
- **elide-llm:** LLM-mediated entity recognizer (text NER and image VLM)
- **elide-backend:** Backend decorators: batching, rate limiting, routing,
  and failover
- **elide-redaction:** The operators, the `Anonymizer` and `Deanonymizer`
  that select and apply them, the key vault, and pseudonym generators
- **elide-fake:** Locale-aware fake-data operator
- **elide-engine:** Document orchestration across multi-part containers
- **elide-codec:** Codec traits and the shared extract engine
- **elide-format:** Format registry over the built-in formats, with
  magic-byte sniffing
- **elide-plain:** TXT, JSON, and CSV handlers
- **elide-markup:** HTML and XML handlers, the markup sanitizer, and character
  reference decoding
- **elide-office:** DOCX, XLSX, and PPTX extraction and byte-faithful redaction
- **elide-pdf:** PDF glyph-deletion and raster redaction of text, images, and
  attachments
- **elide-image:** Raster image decode and encode, EXIF read and strip, and
  pixel-region redaction, with OCR enrichment
- **elide-audio:** Audio decode and encode and time-span redaction, with
  speech-to-text enrichment
- **elide-wasm:** WebAssembly bindings for the browser

[Unreleased]: https://github.com/nvisycom/elide/commits/main
