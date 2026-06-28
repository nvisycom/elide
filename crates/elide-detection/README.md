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
use elide_detection::layer::reconcile::ReconcileLayer;
use elide_detection::layer::reconcile::{Merging, Structural};

let entities = Analyzer::new()
    .with_recognizer(us_phone)
    .with_recognizer(ner)
    // Fuse same-label overlaps into one entity...
    .with_layer(ReconcileLayer::same_label(Merging::max()))
    // ...then arbitrate cross-label overlaps.
    .with_layer(ReconcileLayer::cross_label(Structural::standard()))
    .analyze(data, &scope)
    .await?;
```

The `layer` module ships the reduce stages, each a `Layer` run in order after
recognition:

- **calibrate** — scale each entity's confidence by a per-recognizer multiplier,
  so detectors with different score distributions are comparable before
  reconciliation.
- **reconcile** — decide what happens to overlapping entities. One layer with
  two axes: a `GroupPredicate` (`G`) chooses *which* entities cluster
  (`SameLabel` same-label, `CrossLabel` cross-label), and a
  `Reconciler` (`R`) chooses *what to do* with each pair:
  - **`Merging`** — combine co-located same-label findings into the union of
    their spans, pooling confidence (`Merging::max()` or `Merging::noisy_or()`).
  - **`Structural`** — geometry-aware cross-label handling: keep legitimate
    nestings, drop subsumed junk, and either auto-resolve a true conflict
    (`Structural::standard()`) or flag it for human review
    (`Structural::reviewing()`).
  - **`Exclusive`** — one finding per span; **`Permissive`** — keep every
    overlap (Presidio-style), deferring to the edit step.
- **filter** — drop entities outside an allow-list of labels or below a
  confidence threshold.

The engine is modality-generic: the same `Analyzer<M>` drives text, tabular,
image, and audio detection.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
