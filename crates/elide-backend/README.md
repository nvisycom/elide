# elide-backend

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Decorators for Elide backends: usage metering, observability sinks,
rate-limiting, routing, and failover.

## Overview

Every model-backed component in the toolkit is a `Backend` (the contract lives in
`elide-core`). This crate owns the *decorators* of that contract — wrappers that
are themselves a `Backend`, so they drop in transparently wherever the inner one
goes, leaving its request and response untouched.

The `metrics` module is always available (it pulls no extra dependency). A
`Metered<B>` wrapper reports each call's usage (the model, its wall-clock time,
the tokens it spent, how much it produced, and whether it failed) to a
`UsageSink`.

The `limit` module bounds how a provider is called, at the one place every call
to it passes through, rather than at each fan-out site above. `BatchChunked<B>`
caps the size of one batch, splitting an oversized `call_batch` into
provider-sized sub-batches. With feature `ratelimit`, `ConcurrencyLimited<B>`
caps *simultaneous* calls with a semaphore — how many run at the provider at
once — so a burst waits instead of opening N connections at once, and
`RateLimited<B>` caps calls *over time* to a `Rate`, with a token bucket that
refills at a steady rate and absorbs a burst up to its capacity, matching a
provider's requests-per-interval quota. The limits are different and compose,
with each other and with `Metered`.

The `routing` module picks *which* backend serves a request.
`LanguageRouted<B>` dispatches each request to a backend registered for its
language, the call's primary language whether caller-asserted or detected
(exact tag first, then primary language subtag), and
falls back to a default backend for unhinted or unrouted languages. A mixed
batch is split by route and the responses reassembled in request order. Every
request exposes its language through `BackendRequest::language` (NER, STT, and
OCR requests carry one; others route to the fallback), so the router works for
any backend. `FallbackRouted<P, F>` calls a primary backend and, when it fails
with an error another backend could avoid (transport, provider, or an
unsupported capability), routes the same request to a fallback, so one
provider's outage is served by another rather than failing the analysis. A
router is not a provider: meter the backends it chooses between, not the
router, and every call is charged to the model that served it.

`BackendExt` applies every decorator by method, so a stack reads in the order
it wraps:

```rust,ignore
use elide_backend::BackendExt;
use elide_backend::limit::Rate;

let backend = multilingual
    .metered(usage.clone())
    .language_routed()
    .route("pl".parse()?, polish.metered(usage.clone()))
    .rate_limited(Rate::per_minute(600))
    .concurrency_limited(8);
```

For per-run usage, `UsageCollector` is the in-memory sink: build one per request,
thread it into that run's metered backends, and read its total at the end. It is
correct under concurrency because each run owns its collector, so a webserver
attributes usage to the right request with no shared state.

For *global* observability, two `UsageSink`s forward each call's usage to the
process-wide ecosystems a deployment already runs. `TracingSink` (feature
`tracing`) emits a `tracing` event per call on the `elide::usage` target, so
usage lands in the subscriber alongside the backends' own spans. `MetricsSink`
(feature `recorder`) records call/duration/output/token metrics through the
`metrics` facade, labelled by model, version, and outcome, for a Prometheus/StatsD
recorder. Both are aggregate and global by design; per-request attribution stays
with a per-run `UsageCollector`. A `Tee` feeds two sinks at once.

```rust,ignore
use elide_backend::BackendExt;
use elide_backend::metrics::{Tee, TracingSink, UsageCollector};

// A run's usage reaches both a per-run collector and the global tracing
// subscriber at once.
let collector = UsageCollector::new();
let backend = my_backend.metered(Tee::new(collector.clone(), TracingSink::new()));
// ... run the analysis with `backend`, then read this run's usage:
let units = collector.total_units();
let tokens = units.tokens();
```

The `model` and `version` labels are a bounded set — the models a deployment
configures — so they are safe as metric labels; per-request identifiers are
deliberately not labelled, as they would explode metric cardinality. That is why
per-run usage lives in an owned `UsageCollector`, not in a global label.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
