# elide-lingua

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Lingua-backed language detection for PII/PHI pipelines.

## Overview

Some recognition is language-aware: a pattern set scoped to one language, a
context keyword list in German, a recognizer that only fires on English text.
Those stages need the document's language before they run. This crate
provides `LinguaEnricher`, an enricher that detects the languages of a piece
of text with the [`lingua`](https://crates.io/crates/lingua) crate and adds
them to the subject for the stages that follow.

Detection is per region: mixed-language input gets one detected language per
region, each with its span and a confidence scored on that region alone;
monolingual input gets one covering the whole text. A caller that asserts
languages overrides detection: one asserted language skips it, and two or
more restrict it to the asserted set.

```rust
use elide_core::enrichment::Enricher;
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Context, Scope, Subject};
use elide_lingua::LinguaEnricher;

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> elide_core::Result<()> {
// Every compiled-in language, or `restricted` to a few for speed and precision.
let enricher = LinguaEnricher::restricted(["en".parse()?])?;

let scope = Scope::new();
let ctx = Context::new(&scope);
let mut subject = Subject::<Text>::new(TextData::new("The quick brown fox jumps over the lazy dog."));
enricher.enrich(&mut subject, &ctx).await?;
assert_eq!(ctx.languages(&subject).primary().unwrap().primary_subtag(), "en");
# Ok(())
# }
```

The candidate languages are fixed at construction, and the detector is built
once and shared by the enricher's clones. Which languages exist at all is a
compile-time choice: English by default, the major world languages with the
`common` feature, and every language `lingua` ships with `all`.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
