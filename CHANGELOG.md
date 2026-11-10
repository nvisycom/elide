# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Multimodal detection-and-redaction toolkit for PII/PHI across text, images,
  audio, and tabular data
- File-format codecs for plain-text, JSON, HTML, XML, DOCX, PDF, RTF, PNG, JPEG,
  TIFF, WAV, MP3, CSV, and XLSX
- Pattern-based entity detection with regex and dictionary recognizers
- Post-match validators (Luhn checksum, SSN format) to reduce false positives
- Built-in dictionaries for nationalities, religions, currencies,
  cryptocurrencies, and languages
- Language detection and keyword-boost context enhancement over recognized
  entities
- LLM/VLM-mediated recognizers (text NER and image VLM) via a pluggable rig
  backend
- Speech-to-text and OCR enrichers that lift audio and images into a
  text-recognizable surface
- Composable recognizer/redaction registries, deduplication layers, and
  validation checks
- Coreference-aware pseudonymization with a vault for stable replacements
- Optional PDF page rendering to images via the native PDFium library
  (`pdf-render` feature)
- Hidden-content sanitizing: every codec strips metadata, comments and
  tracked revisions, previews, external references, active content, and
  editor fingerprints at decode, under a per-document `HiddenContentPolicy`
  whose metadata and collaboration `Disposition` is `KeepOnlyEssential`,
  `ScanAsContent`, or `FlagEverything`;
  embedded documents are decoded under their parent's policy, every encode
  checks the output carries none, and an office document with an
  unrecognized part or XML vocabulary is refused
- PDF sanitizing refuses a document encrypted with a user password, keeps an
  allowlist of catalog, page, and trailer entries (removing signature data,
  web-capture and portfolio entries, page timestamps, and page labels),
  strips marked content's alternate text from the content streams, reduces
  kept internal links to their navigation and removes ones that chain an
  action, and gives embedded files neutral names
- PDF images stored as raw samples (every lossless image) are decoded and
  surfaced to the image pipeline as PNG, JPEG images surface in every build,
  and images that cannot be surfaced (fax, JBIG2, stencil masks, inline
  images) are removed rather than shipped unscanned; an image or scanned page
  no codec can read is removed or cleared instead of replaced with a blank
- Office documents surface the text they keep in attributes and outside a
  workbook's cells: comment and change authors and a comment's title (under
  collaboration `ScanAsContent`), sheet, table, table-column, and defined names and
  Word bookmarks (fitted to their format's naming rules and kept unique when
  redacted, with the hyperlinks to them following), dropdown entries,
  data-validation messages, section names, worksheet headers and footers,
  validation lists, formulas, defined-name formulas, and phonetic readings;
  they strip alternative text, tooltips, form-field and content-control
  labels, and optional collaboration dates, set required ones to a neutral
  date, always strip a workbook's revision log, drop a stripped comment's
  legacy note shape, and accept Excel threaded comments and PowerPoint modern
  comments under `ScanAsContent`
- HTML sanitizing reads URLs the way a browser does (character references
  decoded, tabs and line breaks dropped), and also strips frames, `<meta>`
  redirects and cookies, conditional comments, remote resource loads and
  pings, `<template>` and `hidden` elements; XML strips `xml-stylesheet` and
  XInclude references and refuses an external DTD
- Metadata is built into every orchestrator: each codec labels the fields it
  surfaces by its format's schema (`MetadataData::label`, required), and the
  label is the field's detection, so a field whose label the request's
  catalog declares is redacted with nothing registered, replacing the
  per-format `ExifRecognizer` and `DocPropsRecognizer`; under
  `ScanAsContent` an unlabelled field is stripped at decode. Selected fields
  are erased by default, or treated by a caller's `Anonymizer<Metadata>`:
  `Replace` and `Mask` overwrite a field that holds text. An image's EXIF and
  an office document's properties are metadata streams of their document,
  not sub-parts decoded through a format of their own
- `Metadata` is `TextRecognizable`: a metadata analyzer's recognizers read
  each field's value, and a `MetadataLocation` narrowed to a byte range of the
  value (`with_span`) is spliced into the field when written back, so a
  finding inside a free-text field is redacted and the rest of the field
  kept, and a field left blank is removed. `MetadataReplacement` is a text
  treatment of the value (`Value(TextReplacement)`) or `Unchanged`, and every
  text operator a tabular cell takes treats a field's value too. An
  office document now surfaces its keywords, category, template path,
  hyperlink base, each entry of `TitlesOfParts` and `HeadingPairs`, and its
  custom properties
  (`docProps/custom.xml`) as free-text fields instead of stripping them.
  `ImageBuffer::edit_metadata` takes each field's new value, and a metadata
  stream keeps its fields in `elide_codec::metadata::MetadataFields`, which
  reads, redacts, and reports them. A docProps key is unique in its part: a
  repeated element is keyed `name#2`, the part lists `TitlesOfParts/<n>` and
  `HeadingPairs/<n>`, and a custom property by its position, `property/<n>`,
  with its name a field of its own (`property/<n>/name`) that is scanned and
  rewritten like its value
- Under `ScanAsContent` an image keeps only the EXIF tags a surfaced field is
  read from: the members of a field's group no field reads (the lens tags,
  destination coordinates, altitude, modify dates, time offsets) are stripped
  at decode instead of following the field's decision unscanned. A capture
  time is read from `DateTimeOriginal` alone, so `CreateDate` is stripped
  like `ModifyDate` instead of standing in for a missing capture time
- Hidden-content dispositions are `KeepOnlyEssential` (the default),
  `ScanAsContent`, and `FlagEverything`, which also makes every item a format
  labels an entity whatever the catalog declares: each metadata field, and
  each comment, comment or revision author, and tracked deletion of an office
  document, each markup comment, and each HTML `<meta>` field, redacted
  whole; a stream reports its labelled hidden content through
  `Stream::flagged`, and the audit records it as a `Flagged` event naming its
  kind, replacing the metadata-only event
- Every orchestrator drives text out of the box, with no recognizers, so
  flagged hidden content in a text stream is erased with nothing registered
- Every pipeline's anonymizer erases a flagged entity no rule covers, so
  flagged hidden content is never kept for want of a rule; registering a
  pipeline requires `Erase: Operator<M>`, and `Keep` and the new
  `MetadataReplacement::Unchanged` leave a metadata field as it is; an EXIF
  field goes with its group when another of the group is redacted, though its
  audit still shows the `Keep`; a finding inside a kept flagged item takes the
  whole item
- Office documents always lose their XML comments, and a workbook with a legacy
  VML drawing, whose root has no namespace, is no longer refused at encode
- An EXIF tag of a logical group (a destination coordinate, a lens, a modify
  date) is stripped when the image carries no field of that group, since it
  could not reach detection
- HTML `<meta>` names are read with character references decoded, so an encoded
  redirect is removed; a tag naming a field is metadata even when it also sets
  a character set or rendering directive, and a character set does not keep a
  `content` beside it; a tag repeating a classifying attribute is metadata too,
  and every `content` it carries is flagged
- Overwriting an EXIF field with blank text removes it
- Hidden-content audit: every stripped item is recorded on the `Report`
  (`stripped()`) by part, item, and count in a format-neutral
  `HiddenContent` vocabulary, filled by `analyze` and replaced by what
  `anonymize_with` saw, and serialized (output only) as the report's
  `stripped` section
- HTML and XML have their own engine crate, `elide-markup`, which also owns
  character reference decoding (`decode_references`, `CharacterReferences`)
  for any format that reads XML; their constructors move from
  `elide::codec::plain` to `elide::codec::markup`, and `elide-plain` keeps
  TXT, JSON, and CSV, one module per format
- `elide-operator` is merged into `elide-redaction`: the operators, vault,
  and generators are `elide_redaction::{operators, vault, generator}`, beside
  the `Anonymizer` and `Deanonymizer`; the facade's `elide::redaction` paths
  are unchanged
- The recognizer decorators share one module per decorator under
  `elide::recognition::decorator`: `resilient`, `chunked`, and `escalation`
- `TabularLocation` narrows to part of a cell with one `span:
  Option<Range<usize>>` (`with_span`), like `TextLocation`, replacing
  `start_offset`/`end_offset` and `with_range`

### Crates

- **elide-core:** Domain types, traits, and errors for the toolkit
- **elide-pattern:** Pattern and dictionary recognizers for PII/PHI detection,
  with keyword-boost context enhancement
- **elide-ner:** NER traits and language detection
- **elide-llm:** LLM-mediated entity recognizer (text NER + image VLM)
- **elide-stt:** Speech-to-text backends and transcription types
- **elide-ocr:** OCR backends and recognized-text types
- **elide-codec:** Codec traits and format handlers for reading and redacting
  documents
- **elide-markup:** HTML and XML format handlers, the markup sanitizer, and
  character reference decoding
- **elide:** Composable component library for pipelines: recognizers, dedup
  layers, checks, redaction strategies

[Unreleased]: https://github.com/nvisycom/elide/commits/main
