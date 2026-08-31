<div align="center">

<img src=".github/assets/logo.png" alt="Elide" width="104" height="104" />

# Elide

**Composable, multimodal toolkit for detecting and redacting sensitive data.**

The building blocks for finding and removing PII and PHI across your documents:
recognizers, deduplication, redaction operators, and format codecs.

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)
[![Security](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/security.yml?branch=main&label=security&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/security.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue?style=flat-square)](LICENSE.txt)

[**nvisy.com**](https://nvisy.com) · [**docs.nvisy.com**](https://docs.nvisy.com)

</div>

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

- **Multimodal:** One entity model spanning text, images, audio, and tabular data, so a recognizer or operator written once serves every format.
- **Detection:** Regex, dictionary, and checksum recognizers with validators, plus NER and LLM/VLM recognition. Language, OCR, and speech enrichers feed the text they produce back into the same pipeline.
- **Context-aware scoring:** Nearby keywords lift ambiguous matches, and overlapping findings reconcile into one deduplicated entity set.
- **Redaction operators:** Mask, replace, truncate, HMAC, hash, generalize, or clamp text; blur, pixelate, or black out image regions; silence or beep audio; drop rows and columns. Reversible encryption and pseudonymization round-trip.
- **Format codecs:** Read and rewrite TXT, CSV, JSON, XML, HTML, RTF, PDF, DOCX, PPTX, XLSX, images (PNG, JPEG, TIFF), and audio (WAV, MP3), changing only the redacted spans and leaving every other byte intact.
- **Provenance-first:** Every entity carries its full audit trail of how it was found, scored, and hidden, and the trail verifies.

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
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
