# Elide

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Composable, multimodal toolkit for detecting and redacting sensitive data.

Elide is a Rust toolkit for finding and removing PII and PHI from text, images,
audio, and tabular data. It provides the building blocks (recognizers,
deduplication, validation, redaction operators, and format codecs) that a
consumer wires into their own document-processing flow. Elide is the toolkit
layer only; the orchestrating runtime and gateway server live in separate
projects.

> [!WARNING]
> **Active development: API not stable.** This project is under active
> development. Public APIs, configuration shapes, and on-disk formats may change
> without notice between releases. Pin a specific commit if you depend on this
> in production.

## Features

- **Multimodal**: detect and redact across text, images, audio, and tabular data
  through one entity model; OCR and speech-to-text lift images and audio into a
  text-recognizable surface so the same recognizers apply
- **Pattern detection**: regex, dictionary, and checksum recognizers find
  structured PII and PHI across many common formats and jurisdictions
- **Model-driven recognition**: NER with language detection, and LLM/VLM
  recognizers (text and image) behind a pluggable backend, alongside the pattern
  recognizers
- **Context-aware scoring**: nearby keywords lift the confidence of ambiguous
  matches, so weak findings clear the threshold only when their surroundings
  support them
- **Deduplication**: overlapping findings from multiple recognizers reconcile
  into a single set of entities, with conflict resolution and confidence
  calibration
- **Redaction operators**: mask, replace, hash, or encrypt text; blur or
  black-box image regions; silence or beep audio; drop tabular rows and columns,
  and more. Reversible options (encrypt, pseudonymize) record what is needed to
  restore the original
- **Format codecs**: read, edit, and write documents (text, JSON, HTML, DOCX,
  PDF, images, audio, CSV, and more) with faithful round-tripping that changes
  only the redacted parts
- **Provenance-first model**: every entity carries its full audit trail of how
  it was found, scored, and hidden

Everything is feature-gated: take only the modalities, recognizers, and codecs
you need.

## Documentation

See [`docs/`](docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
