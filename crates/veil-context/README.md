# nvisy-context

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/runtime/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/runtime/actions/workflows/build.yml)

Post-recognition keyword-boost enhancer for the Nvisy runtime.

## Overview

Context-aware confidence boosting. Every recognizer that wants
score boosting declares a `Context` (a list of keywords
plus optional window / boost overrides), registered against the
recognizer's name. After recognition, `ContextEnhancer` walks each
detected `Entity<Text>`, looks the recognizer name up in the
`ContextRegistry`, scans the surrounding window for any declared
keyword via the configured `KeywordMatcher`, and bumps the entity's
confidence on a hit.

`Tokens` is the optional NLP artifact (surface + lemma per token)
that a tokenizing NLP engine stashes on `RecognizerInput.artifacts`
so `LemmaMatcher` can match morphological variants (`running` →
`run`). The `SubstringMatcher` fallback runs whenever no `Tokens`
artifact is present.

The crate depends only on `nvisy-core` for `Entity<Text>`,
`TrailStep`, and `Confidence` — recognizer crates and the engine
each depend on `nvisy-context` to participate.

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
