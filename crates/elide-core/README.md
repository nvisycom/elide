# elide-core

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Shared domain model, traits, and errors for the elide toolkit.

## Overview

The foundational crate of the workspace. It owns the shared vocabulary
that every other crate speaks: what a detected entity is, where it sits
in a document, how confident a detection is, and the traits that
recognizers and redaction operators implement. It carries no
orchestration and no concrete recognizers or operators; those live in
the downstream crates.

The model draws from [Presidio](https://github.com/microsoft/presidio)
but is shaped by two goals that set it apart:

- **Multimodal by construction.** Nothing in the core hardcodes text
  offsets. An entity's location is defined by its medium (text, image,
  audio, …), and the core types are generic over that medium. Each
  modality lives in its own crate, so new media need no change here.

- **Provenance-first.** Every entity carries its full audit trail: which
  recognizers found it, how overlapping findings were combined, any
  confidence adjustments, and every redaction applied. Nothing about how
  an entity was found, scored, or hidden is ever discarded.

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
