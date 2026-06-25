# elide-lingua

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Lingua-backed language detection for PII/PHI pipelines.

## Overview

Some recognition is language-aware: a pattern set scoped to one language, a
context enhancer whose keywords are German, a recognizer that only fires on
English text. Those stages need to know the document's language before they
run. This crate provides `LinguaEnricher`, an [`Enricher`] that detects the
language(s) of a piece of text — backed by the [`lingua`](https://crates.io/crates/lingua)
crate — and stamps the result onto the call for the stages that follow.

An enricher runs ahead of the recognizers and the context enhancer: it resolves
the languages onto the input, and downstream stages read them back. Detection is
per-region, so mixed-language input is attributed one detection per detected
span; monolingual input yields a single detection covering the whole text. When
the caller has already asserted a language on the input, detection is skipped —
the assertion is authoritative.

```rust,ignore
use elide_lingua::LinguaEnricher;

// Detect across every language compiled into the lingua feature set,
// or restrict the candidate set for speed.
let enricher = LinguaEnricher::unrestricted();
```

The candidate scope is fixed at construction; a pipeline that needs different
scopes per call holds multiple enrichers. Each call builds a fresh detector, so
the enricher itself is stateless and cheap to clone.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

[`Enricher`]: https://docs.rs/elide-core/latest/elide_core/recognition/trait.Enricher.html
