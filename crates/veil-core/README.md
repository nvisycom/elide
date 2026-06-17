# veil-core

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/veil/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/veil/actions/workflows/build.yml)

Domain types, traits, and errors for the Veil toolkit.

## Overview

The foundational crate of the workspace. Owns the shared domain model
— entities, spans, modalities — and the recognition/redaction traits
the rest of the toolkit builds on. Carries no orchestration or concrete
recognizer/operator implementations; those live in downstream crates
(`veil-pattern`, `veil-toolkit`, …).

The model draws from [Presidio](https://github.com/microsoft/presidio)
but is shaped by two goals that set it apart:

- **Multimodal by construction.** Nothing in the core hardcodes text
  offsets. Locations are expressed through the `Span` trait, and the
  central types (`Entity<S>`, `Detection<S>`) are generic over the span
  type. Text, image, audio, and document spans are defined in their
  respective modality crates; new modalities need no change here.

- **Provenance-first.** Every `Entity` carries a `Provenance`: the full,
  always-present audit trail of every detection layer that found it
  (`detections`), how those detections were combined (`merge`), and
  every redaction applied (`history` of `Transformation`s) — each event
  versioned and timestamped. A run-level `Manifest` anchors it all to
  the source hash and engine build.

### Flow

`Recognizer`s emit `Detection`s → a `MergeStrategy` combines overlapping
detections into an `Entity` → `Operator`s transform the entity's
content, each recording a `Transformation`. The entity's `Provenance`
accumulates the whole story, so nothing about how an entity was found,
scored, or redacted is ever lost.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](../../LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/veil/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
