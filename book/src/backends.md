# Backends

Every model Elide calls (a NER model, an LLM, an OCR engine, a
speech-to-text service) sits behind one trait, `Backend`: a request goes
in, the model is called, a response comes out. A recognizer or enricher
holds a backend and drives it; the backend owns the transport.

| Used by | Request | Response | Shipped backends |
|---|---|---|---|
| `NerRecognizer` | `NerRequest` | `NerResponse` | None; bring your own |
| `LlmRecognizer` | `LlmRequest` | `LlmResponse` | `RigBackend` |
| `OcrEnricher` | `OcrRequest` | `OcrResponse` | `OcrsBackend` (`elide-image`, feature `ocrs`) |
| `SttEnricher` | `SttRequest` | `SttResponse` | None; bring your own |

With the `mocks` feature, each has a `MockBackend` that finds nothing, for
tests and offline examples.

A request borrows its payload, so a large image or recording is not copied
to make a call. `call_batch` sends many requests together; a backend whose
provider accepts batches overrides it to make one round trip instead of
many. `provenance` names the model, for the audit trail of every entity it
finds.

## Decorators

Because every model has the same shape, one set of *decorators* serves them
all. A decorator wraps a backend and is a backend itself, with the same
request and response, so it goes anywhere the inner one does. `BackendExt`
applies each by method, so a stack reads in the order it wraps, innermost
first:

```rust
use elide::backend::BackendExt;
use elide::backend::limit::Rate;
use elide::backend::metrics::UsageCollector;
use elide::recognition::ner::NerRecognizer;
# use elide::recognition::ner::backend::MockBackend;

# #[tokio::main]
# async fn main() {
# let backend = MockBackend::new();
let usage = UsageCollector::new();

let backend = backend
    .metered(usage.clone())
    .rate_limited(Rate::per_minute(60).with_burst(10))
    .concurrency_limited(8);

let ner = NerRecognizer::new(backend);
# }
```

Each decorator is also a type with a `new` constructor, such as
`Metered::new(backend, usage)`, for when you want to name it.

| Decorator | Method | Feature | Does |
|---|---|---|---|
| `Metered` | `metered` | `metrics` | Records each call's model, duration, and tokens; see [Usage](usage.md) |
| `BatchChunked` | `batch_chunked` | any backend feature | Splits batches larger than the provider accepts; see [Rate limits](rate-limits.md) |
| `ConcurrencyLimited` | `concurrency_limited` | `ratelimit` | Caps calls in flight at once |
| `RateLimited` | `rate_limited` | `ratelimit` | Caps calls per interval |
| `LanguageRouted` | `language_routed` | `routing` | Picks a backend by the request's language; see [Language routing](routing.md) |
| `FallbackRouted` | `fallback_routed` | `routing` | Routes a failed call to a fallback backend; see [Failover](failover.md) |

NER backends have three more, for the labels and scores of a model; see
[Named-entity models](ner.md#supported-labels).

To write a backend for your own model, see
[Custom backends](custom-backends.md).
