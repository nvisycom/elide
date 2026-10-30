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
