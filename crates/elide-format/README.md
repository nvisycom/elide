# elide-format

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The format registry that assembles the elide codecs: resolve a file to a handler
and decode it.

## Overview

`elide-codec` defines the codec *contracts* — the `Handler` and `Loader` traits,
the `Format` descriptor, and the format-neutral decode machinery. The concrete
handlers live with their engines, each behind that engine's `codec` feature: the
leaf text-shaped formats (TXT, JSON, HTML, XML, CSV) in `elide-plain`, and the
rich-media formats in `elide-image`, `elide-pdf`, `elide-audio`, and
`elide-office`. This crate is the *assembly* layer that ties them together.

It owns the `FormatRegistry`, which indexes every enabled format by its
identifier, file extension, and content type, and decodes raw bytes through the
matching loader. `FormatRegistry::with_builtin` wires up exactly the formats the
active feature set enables, so a caller resolves a document from a filename or a
MIME type without hardcoding which crate handles it.

Formats are opt-in: enable only the ones a build needs, and register custom
formats on the registry at runtime without changing the core. The registry is a
cheap-to-clone handle over immutable shared state, built once and shared across
an analysis.

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
