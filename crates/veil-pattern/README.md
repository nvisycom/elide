# nvisy-pattern

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/runtime/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/runtime/actions/workflows/build.yml)

Regex and dictionary recognizers for PII / PHI detection in the
Nvisy runtime.

## Overview

`PatternRecognizer` compiles a set of `Regex` rules (each holding
one or more regex `Variant`s grouped as a multi-strategy detector
for one entity type) and `Dictionary` term lists into pooled
scanners — one shared `regex::RegexSet` for the regex side and
one shared `aho_corasick::AhoCorasick` automaton for the literal
side. A single walk over the input runs both scanners and emits
`Entity<Text>` values in modality-local byte coordinates.

Rules may declare per-label context keywords. Calling
`build_context_enhanced()` wraps the recognizer in a
`nvisy_context::ContextEnhanced` layer that lifts confidence on
matches whose neighbourhood contains a declared keyword;
`build()` returns the bare recognizer.

The built-in pattern + dictionary set lives as TOML under
`assets/` and is embedded at compile time. The recognizer's
builder accepts both built-ins and user-supplied rules:

```rust
use nvisy_pattern::PatternRecognizer;

let recognizer = PatternRecognizer::builder()
    .with_builtin_patterns()
    .with_builtin_dictionaries()
    .build()
    .expect("built-in recognizer builds");
```

Regex variants can opt into a post-match validator by name
(`"luhn"`, `"ssn"`, `"iban"`, `"phone"`, `"date"`); custom
validators can be registered via `ValidatorRegistry::with`.

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
