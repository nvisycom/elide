<div align="center">

<img src=".github/assets/logo.png" alt="Elide" width="104" height="104" />

# Elide

**Composable, multimodal toolkit for detecting and redacting sensitive data.**

The building blocks — recognizers, deduplication, redaction operators, and
format codecs — for finding and removing PII and PHI across your documents.

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

- **Multimodal:** Detect and redact across text, images, audio, and tabular data through one entity model
- **Pattern & Model Detection:** Regex, dictionary, and checksum recognizers alongside NER and LLM/VLM recognition
- **Context-Aware Scoring:** Nearby keywords lift ambiguous matches, and overlapping findings deduplicate into one entity set
- **Redaction Operators:** Mask, replace, hash, encrypt, blur, silence, or drop — with reversible encrypt and pseudonymize
- **Format Codecs:** Round-trip PDF, DOCX, HTML, JSON, CSV, images, and audio, changing only the redacted parts
- **Provenance-First:** Every entity carries its full audit trail of how it was found, scored, and hidden

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
