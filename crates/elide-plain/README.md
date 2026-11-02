# elide-plain

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The leaf plain-text format handlers: plain text, JSON, and CSV.

## Overview

Most formats a redaction toolkit handles are backed by a dedicated engine — an
image decoder, an audio codec, a PDF parser, a markup reader. The plain-text
formats are the exception: TXT, JSON, and CSV are all fundamentally text, and
their handlers need no engine of their own, only the codec contracts and the
shared string helpers. This crate is their home; HTML and XML live in
`elide-markup`.

Each format ships a `Handler` + `Loader` pair behind a `*_format()` constructor,
implemented against the traits in `elide-codec`. The handlers read a document's
text so recognizers can scan it, then splice redactions back into the exact
source positions and re-encode — faithfully, so only the redacted spans change.
JSON maps each decoded string back to its raw source offsets; CSV redacts cells
as text.

These constructors are the crate's public surface. `elide-format`'s
`FormatRegistry::with_builtin` wires the enabled ones into the registry alongside
the formats from the engine crates. Formats are opt-in: enable only the ones a
build needs.

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
