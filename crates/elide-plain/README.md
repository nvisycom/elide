# elide-plain

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The leaf text-shaped format handlers: plain text, JSON, HTML, XML, and CSV.

## Overview

Most formats a redaction toolkit handles are backed by a dedicated engine — an
image decoder, an audio codec, a PDF parser. The text-shaped formats are the
exception: TXT, JSON, HTML, XML, and CSV are all fundamentally text, and their
handlers need no binary engine, only the codec contracts and the shared
text-extraction machinery. This crate is their home.

Each format ships a `Handler` + `Loader` pair behind a `*_format()` constructor,
implemented against the traits in `elide-codec`. The handlers read a document's
text so recognizers can scan it, then splice redactions back into the exact
source positions and re-encode — faithfully, so only the redacted spans change.
The structured formats (JSON, HTML, XML) reuse the shared extract-and-splice
engine to map decoded text back to raw source offsets; CSV redacts cells as
text.

These constructors are the crate's public surface. `elide-format`'s
`FormatRegistry::with_builtin` wires the enabled ones into the registry alongside
the rich-media formats from the other engine crates. Formats are opt-in: enable
only the ones a build needs.

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
