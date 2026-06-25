# elide-detection

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The detection engine for PII/PHI: the `Analyzer` and its deduplication layers.

## Overview

Finding sensitive data is more than running one recognizer. A real pipeline
enriches the input (detecting its language, say), runs several recognizers that
each emit entities independently, then reconciles those overlapping, duplicated,
differently-scored findings into one clean set. This crate is that engine.

`Analyzer` is the Presidio-style "find" entry point. Enrichers, recognizers, and
deduplication layers are added with `with_*` builders, each in the order it
should run; `analyze` then runs three phases — enrich (sequential), recognize
(concurrent), reduce (the layers) — and returns the reconciled entities.

```rust,ignore
use elide_detection::Analyzer;
use elide_detection::deduplication::fuse::FuseLayer;

let entities = Analyzer::new()
    .with_recognizer(us_phone)
    .with_recognizer(ner)
    .with_layer(FuseLayer::new(MaxConfidence))
    .analyze(data, &scope)
    .await?;
```

The `deduplication` module ships the reduce stages, each a `Layer` run in order
after recognition:

- **calibrate** — scale each entity's confidence by a per-recognizer multiplier,
  so detectors with different score distributions are comparable before fusion.
- **fuse** — combine co-located findings of the *same* label into one entity,
  accumulating their detections in the survivor's provenance.
- **resolve** — break overlaps between *different* labels, dropping the loser.
- **filter** — drop entities outside an allow-list of labels or below a
  confidence threshold.

The engine is modality-generic: the same `Analyzer<M>` drives text, tabular,
image, and audio detection.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
