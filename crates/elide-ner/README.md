# elide-ner

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Named-entity recognition for PII/PHI detection.

## Overview

Some sensitive data has no fixed shape: a person's name, an organization, a
place. Recognizing it takes a model that reads the surrounding language rather
than a regular expression. This crate provides the recognizer that turns
model-produced spans into typed entities, with a pluggable backend so the model
itself can run wherever suits the deployment: in process, as a hosted service,
or as a future local inference engine. A no-op backend ships built in for wiring
and tests, and concrete inference backends live downstream.

Raw backend labels are projected onto the toolkit's canonical label set, so
consumers reason about one fixed taxonomy regardless of the upstream model.
Language detection — resolving the language of a piece of text for language-aware
recognizers and policies — lives in the separate `elide-lingua` crate.

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
