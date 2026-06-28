# elide

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Composable, multimodal toolkit for detecting and redacting sensitive data.

## Overview

Elide is a Rust toolkit for finding and removing PII and PHI from text, images,
audio, and tabular data. It runs a set of recognizers over content — regex,
dictionary, and checksum patterns; NER and LLM/VLM models behind pluggable
backends — and reconciles their overlapping findings into a single set of
entities, resolving conflicts, calibrating confidence, and dropping weak
matches. It then applies redaction operators that hide each entity in a chosen
way: mask, replace, hash, or encrypt text; blur or black-box image regions;
silence audio; drop tabular rows and columns. Format codecs read, edit, and
write whole documents (text, JSON, HTML, DOCX, images, audio, …), and the
orchestrator drives the flow across the body and embedded parts of a
multi-modal container.

OCR and speech-to-text lift images and audio onto a text-recognizable surface,
so the same recognizers apply across every modality through one entity model.
Recognition, detection, redaction, and orchestration are independent engine
crates; this crate is the umbrella that gathers them — and `elide-core` —
behind one import, feature-gated so a consumer pulls in only what they use.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](../../LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
