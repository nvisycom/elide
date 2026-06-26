# elide

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The umbrella facade: recognition, detection, redaction, and orchestration in
one crate.

## Overview

`elide` is a thin re-export facade over the toolkit's engine crates — the way
the `burn` crate re-exports `burn-core`, `burn-tensor`, and friends. It runs a
set of recognizers over content, reconciles their overlapping findings into a
single set of entities (resolving conflicts, adjusting confidence, and dropping
weak matches), and applies redaction operators that hide each entity in a chosen
way, such as masking, replacing, hashing, or encrypting it — and, with the
`codec` feature, drives that whole flow across multi-modal documents.

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
