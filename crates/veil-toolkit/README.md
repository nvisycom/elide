# veil-toolkit

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/veil/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/veil/actions/workflows/build.yml)

Composable component library for Veil pipelines — the registries and
policies a consumer plugs into their own document-processing flow.

## Overview

Hosts the per-stage component machinery a document orchestrator drives:
recognizer and redaction registries, deduplication layers, and
validation checks. Sits one level above [`veil-core`](../veil-core):
the toolkit owns reusable pieces; the orchestration that strings them
into a full pipeline lives one layer up.

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
