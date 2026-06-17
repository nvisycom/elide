# veil-ner

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/veil/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/veil/actions/workflows/build.yml)

Named-entity recognition and language detection for PII/PHI detection.

## Overview

Some sensitive data has no fixed shape: a person's name, an
organization, a place. Recognizing it takes a model that reads the
surrounding language rather than a regular expression. This crate
provides the recognizer that turns model-produced spans into typed
entities, with a pluggable backend so the model itself can run wherever
suits the deployment: in process, as a hosted service, or as a future
local inference engine. A no-op backend ships built in for wiring and
tests, and concrete inference backends live downstream.

Raw backend labels are projected onto the toolkit's canonical label set,
so consumers reason about one fixed taxonomy regardless of the upstream
model. The crate also ships language detection, which resolves the
language of a piece of text and carries that result alongside the input
for language-aware recognizers and policies.

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
