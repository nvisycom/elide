# elide-codec

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The codec contracts for reading and redacting documents across file formats.

## Overview

Detection works on text, but real documents arrive as files: plain text, JSON,
HTML, XML, images, audio, PDFs, and Office documents. This crate defines the
contracts that bridge the two. The `Handler` and `Loader` traits describe how a
format decodes its bytes, exposes its content so recognizers can scan it, applies
redactions back to the right places, and re-encodes; the `Format` descriptor and
the `Container`/`Part` surface for documents that nest sub-parts of other
modalities round out the vocabulary.

It ships no format handlers of its own. Each format is implemented against these
traits in the crate that owns its engine — the text-shaped formats in
`elide-plain`, and images, audio, PDF, and Office documents in `elide-image`,
`elide-audio`, `elide-pdf`, and `elide-office` — and `elide-format` assembles
them into a registry. Alongside the traits, this crate carries the shared
text-extraction machinery those handlers build on: the extract-and-splice engine
that maps a decoded value back to its raw source offsets, and the byte-level
redaction helper.

The guiding principle is faithful round-tripping. When a document is re-encoded,
only the redacted parts change; structure, formatting, and everything left
untouched are preserved.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE](../../LICENSE)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
