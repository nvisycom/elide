# elide-codec

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The codec contracts for reading and redacting documents across file formats.

## Overview

Detection works on text, but real documents arrive as files: plain text, JSON,
HTML, XML, images, audio, PDFs, and Office documents. This crate defines the
contracts that bridge the two. A format's `DocumentLoader` decodes its bytes
into a `Document`: the format's `Native` document, which owns the document's
`Stream`s, one per modality it carries (a text body, an image's pixels, an
audio clip), and the blobs embedded in it, each a file decoded as a document of
its own. Detection reads the streams and redaction edits them in place; the
native document then writes itself back into native bytes, with the blobs'
redacted bytes, and checks the result. A `Format` names the format and the
extensions and content types it is found by.

It ships no formats of its own. Each format is implemented against these
contracts in the crate that owns its engine: the plain-text formats in
`elide-plain`, HTML and XML in `elide-markup`, and images, audio, PDF, and
Office documents in `elide-image`, `elide-audio`, `elide-pdf`, and
`elide-office`; `elide-format` assembles them into a registry. Alongside the
contracts, this crate carries the shared text-extraction machinery those formats
build on: the extract-and-splice engine that maps a decoded value back to its
raw source offsets, and the byte-level redaction helper.

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
