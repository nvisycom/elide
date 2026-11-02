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
- Name walks: company names ending in a legal form, institutions around a
  head word, and a person's name after a title, with legal forms and word
  lists converted from stella/anonymize
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
  whose metadata and collaboration `Disposition` can be `Redact` instead;
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
  collaboration `Redact`), sheet, table, table-column, and defined names and
  Word bookmarks (fitted to their format's naming rules and kept unique when
  redacted, with the hyperlinks to them following), dropdown entries,
  data-validation messages, section names, worksheet headers and footers,
  validation lists, formulas, defined-name formulas, and phonetic readings;
  they strip alternative text, tooltips, form-field and content-control
  labels, and optional collaboration dates, set required ones to a neutral
  date, always strip a workbook's revision log, drop a stripped comment's
  legacy note shape, and accept Excel threaded comments and PowerPoint modern
  comments under `Redact`
- HTML sanitizing reads URLs the way a browser does (character references
  decoded, tabs and line breaks dropped), and also strips frames, `<meta>`
  redirects and cookies, conditional comments, remote resource loads and
  pings, `<template>` and `hidden` elements; XML strips `xml-stylesheet` and
  XInclude references and refuses an external DTD
- Metadata fields carry what they are: each codec labels the fields it
  surfaces by its format's schema (`MetadataData::label`), and one
  format-neutral `FieldRecognizer` turns them into entities, replacing the
  per-format `ExifRecognizer` and `DocPropsRecognizer`; under `Redact` an
  unlabelled field is stripped, and metadata no pipeline scans is removed and
  reported as `HiddenContent::Unscanned`
- Hidden-content audit: every stripped item is recorded on the `Report`
  (`stripped()`) by part, item, and count in a format-neutral
  `HiddenContent` vocabulary, filled by `analyze` and replaced by what
  `anonymize_with` saw, and serialized (output only) as the report's
  `stripped` section

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
- **elide:** Composable component library for pipelines: recognizers, dedup
  layers, checks, redaction strategies

[Unreleased]: https://github.com/nvisycom/elide/commits/main
