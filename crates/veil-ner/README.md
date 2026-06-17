# nvisy-ner

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/runtime/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/runtime/actions/workflows/build.yml)

NER recognizer + pluggable inference `Backend` trait for the Nvisy
runtime.

## Overview

`NerRecognizer` turns model-produced spans into typed `Entity<Text>`
values via any implementation of the crate's `Backend` trait.
Shipped backends: `NoopBackend` (test stub) and the feature-gated
`BentoBackend`, an HTTP client into the externalised
`inference-gliner` Bento. Model-bearing inference is intentionally
out-of-process — see [`nvisycom/inference`].

`LabelMap` projects raw backend labels onto the canonical
`EntityKind` set so consumers reason about one fixed taxonomy
regardless of the upstream model. LLM-mediated NER lives in
`nvisy-llm`.

[`nvisycom/inference`]: https://github.com/nvisycom/inference

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](../../LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/runtime/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
