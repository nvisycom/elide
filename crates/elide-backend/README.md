# elide-backend

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Decorators for Elide backends: usage metering, observability sinks, and
concurrency rate-limiting.

## Overview

Every model-backed component in the toolkit is a `Backend` (the contract lives in
`elide-core`). This crate owns the *decorators* of that contract — wrappers that
are themselves a `Backend`, so they drop in transparently wherever the inner one
goes, leaving its request and response untouched.

The `metrics` module is always available (it pulls no extra dependency). A
`Metered<B>` wrapper reports each call's usage (the model, its wall-clock time,
the tokens it spent, how much it produced, and whether it failed) to a
`UsageSink`.

The `ratelimit` module (feature `ratelimit`) bounds how a provider is called,
at the one place every call to it passes through, rather than at each fan-out
site above. `ConcurrencyLimited<B>` caps *simultaneous* calls with a semaphore —
how many run at the provider at once — so a burst waits instead of opening N
connections at once. `RateLimited<B>` caps calls *over time* with a token
bucket: the bucket refills at a steady rate and absorbs a burst up to its
capacity, matching a provider's requests-per-interval quota. The two are
different limits and compose — wrap a backend in both to bound its open
connections and its request rate — and both compose with `Metered`.

For per-run cost, `UsageCollector` is the in-memory sink: build one per request,
thread it into that run's metered backends, and read its total at the end. It is
correct under concurrency because each run owns its collector, so a webserver
attributes cost to the right request with no shared state.

For *global* observability, two `UsageSink`s forward each call's usage to the
process-wide ecosystems a deployment already runs. `TracingSink` (feature
`tracing`) emits a `tracing` event per call on the `elide::usage` target, so
usage lands in the subscriber alongside the backends' own spans. `MetricsSink`
(feature `recorder`) records call/duration/output/token metrics through the
`metrics` facade, labelled by model, version, and outcome, for a Prometheus/StatsD
recorder. Both are aggregate and global by design; per-request attribution stays
with a per-run `UsageCollector`. A `Tee` feeds two sinks at once.

```rust,ignore
use std::sync::Arc;
use elide_backend::metrics::{Metered, Tee, TracingSink, UsageCollector, UsageSink};

// A run's usage reaches both a per-run collector and the global tracing
// subscriber at once.
let collector = UsageCollector::new();
let sink: Arc<dyn UsageSink> = Arc::new(Tee::new(collector.clone(), TracingSink::new()));

let backend = Metered::new(my_backend, sink);
// ... run the analysis with `backend`, then read this run's cost:
let units = collector.total_units();
let tokens = units.tokens();
```

The `model` and `version` labels are a bounded set — the models a deployment
configures — so they are safe as metric labels; per-request identifiers are
deliberately not labelled, as they would explode metric cardinality. That is why
per-run cost lives in an owned `UsageCollector`, not in a global label.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
