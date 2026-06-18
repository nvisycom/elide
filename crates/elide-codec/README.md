# elide-codec

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Reading and redacting documents across file formats.

## Overview

Detection works on text, but real documents arrive as files: plain text,
JSON, HTML, XML, and more. This crate bridges the two. It reads a
document of a given format, exposes its text so recognizers can scan it,
applies redactions back to the right places, and writes the document out
again.

The guiding principle is faithful round-tripping. When a document is
re-encoded, only the redacted parts change; structure, formatting, and
everything left untouched are preserved. A registry resolves the right
handler from a file's extension or content type, so callers work with
documents without hardcoding format details.

Formats are opt-in: enable only the ones you need, and add support for
new formats without changing the core. Support beyond plain text is an
ongoing effort.

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
