# elide-backend

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Decorators for Elide backends: batching, rate-limiting, routing, and
failover.

## Overview

Every model-backed component in the toolkit is a `Backend` (the contract lives in
`elide-core`). This crate owns the *decorators* of that contract — wrappers that
are themselves a `Backend`, so they drop in transparently wherever the inner one
goes, leaving its request and response untouched.

The `limit` module bounds how a provider is called, at the one place every call
to it passes through, rather than at each fan-out site above. `BatchChunked<B>`
caps the size of one batch, splitting an oversized `call_batch` into
provider-sized sub-batches. With feature `ratelimit`, `ConcurrencyLimited<B>`
caps *simultaneous* calls with a semaphore — how many run at the provider at
once — so a burst waits instead of opening N connections at once, and
`RateLimited<B>` caps calls *over time* to a `Rate`, with a token bucket that
refills at a steady rate and absorbs a burst up to its capacity, matching a
provider's requests-per-interval quota. The limits are different and compose.

The `routing` module picks *which* backend serves a request.
`LanguageRouted<B>` dispatches each request to a backend registered for its
language, the call's primary language whether caller-asserted or detected
(exact tag first, then primary language subtag), and falls back to a default
backend for unhinted or unrouted languages. A mixed batch is split by route and
the responses reassembled in request order. Every request exposes its language
through `BackendRequest::language` (NER, STT, and OCR requests carry one; others
route to the fallback), so the router works for any backend.
`FallbackRouted<P, F>` calls a primary backend and, when it fails with an error
another backend could avoid (transport, provider, or an unsupported
capability), routes the same request to a fallback, so one provider's outage is
served by another rather than failing the analysis.

`BackendExt` applies every decorator by method, so a stack reads in the order
it wraps:

```rust,ignore
use elide_backend::BackendExt;
use elide_backend::limit::Rate;

let backend = multilingual
    .language_routed()
    .route("pl".parse()?, polish)
    .rate_limited(Rate::per_minute(600))
    .concurrency_limited(8);
```

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
