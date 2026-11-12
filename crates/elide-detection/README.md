# elide-detection

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The detection engine for PII/PHI: the `Analyzer` and its `Layer` pipeline.

## Overview

Finding sensitive data is more than running one recognizer. A real pipeline
enriches the input (detecting its language, say), runs several recognizers that
each emit entities independently, then reconciles those overlapping, duplicated,
differently-scored findings into one clean set. This crate is that engine.

`Analyzer` is the Presidio-style "find" entry point. Enrichers, recognizers, and
layers are added with `with_*` builders, each in the order it should run;
`analyze` then runs three phases — enrich (sequential), recognize (concurrent),
reduce (the layers) — and returns the reconciled entities.

```rust,ignore
use elide_detection::Analyzer;
use elide_detection::overlap::{FuseLayer, ReconcileLayer, Structural};

let analysis = Analyzer::new()
    .with_recognizer(us_phone)
    .with_recognizer(ner)
    // Fuse same-label overlaps into one entity...
    .with_layer(FuseLayer::max())
    // ...then arbitrate cross-label overlaps.
    .with_layer(ReconcileLayer::structural())
    .analyze(data, &scope)
    .await?;
let entities = analysis.entities;
```

The `layer` module ships the reduce stages, each a `Layer` run in order after
recognition:

- **calibrate** — scale each entity's confidence by a per-recognizer multiplier,
  so detectors with different score distributions are comparable before
  reconciliation.
- **overlap** — decide what happens to overlapping entities. A
  `GroupPredicate` chooses *which* entities cluster (`SameLabel` same-label,
  `CrossLabel` cross-label); two layers act on the clusters:
  - **`FuseLayer`** — fuse each cluster of the same finding into one entity
    over the union of their spans, pooling confidence over every member
    (`FuseLayer::max()` or `FuseLayer::noisy_or()`).
  - **`ReconcileLayer`** — arbitrate each pair with a `Reconciler`:
    - **`Structural`** — geometry-aware cross-label handling: keep legitimate
      nestings, drop subsumed junk, and either auto-resolve a true conflict
      (`Structural::standard()`) or flag it for human review
      (`Structural::standard().reviewing()`).
    - **`Exclusive`** — one finding per span; **`Permissive`** — keep every
      overlap (Presidio-style), deferring to the edit step.
- **filter** — drop entities below a confidence threshold.

The engine is modality-generic: the same `Analyzer<M>` drives text, tabular,
image, and audio detection.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
