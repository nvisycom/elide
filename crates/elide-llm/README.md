# elide-llm

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

LLM-driven entity recognition over text and images for PII/PHI detection.

## Overview

Some sensitive data resists fixed rules: free-form prose, a face in a
photo, a license plate on a sign. Recognizing it well means asking a
model that reads meaning rather than matching a pattern. This crate
provides a recognizer that drives a large language model (for text) or a
vision-language model (for images) and turns its replies into typed
entities, located back in the source so they can be redacted.

The model itself is swappable: providers connect to hosted services
(OpenAI, Anthropic, Google) or a local runner, each enabled by an opt-in
feature so consumers pay only for what they use. The prompt is swappable
too: ship the built-in prompt, or load one from a TOML file so wording,
label remapping, and ignore lists become data the operator edits rather
than code. Either way the model's raw labels are projected onto the
toolkit's canonical taxonomy, so downstream code reasons about one fixed
label set regardless of which model produced a finding.

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
