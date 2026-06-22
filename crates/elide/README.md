# elide

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Composable recognition, deduplication, and redaction components.

## Overview

This crate assembles the lower-level pieces into the working parts of a
detection-and-redaction flow. It runs a set of recognizers over content,
reconciles their overlapping findings into a single set of entities (resolving
conflicts, adjusting confidence, and dropping weak matches), and applies
redaction operators that hide each entity in a chosen way, such as masking,
replacing, hashing, or encrypting it.

It provides the reusable building blocks rather than a fixed pipeline. The
orchestration that strings them into an end-to-end flow over whole documents
lives one layer up, in the runtime.

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
