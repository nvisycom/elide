# Rate limits

Providers limit how hard you can call them: so many requests at once, so
many per minute, so many inputs per batch. Elide enforces these limits on
the backend, the one place every call to the provider passes through,
however many recognizers and documents share it. They live in
`elide::backend::limit`; the two that wait on a clock come with the
`ratelimit` feature.

```rust
use elide::backend::BackendExt;
use elide::backend::limit::Rate;
# use elide::recognition::ner::backend::MockBackend;

# #[tokio::main]
# async fn main() {
# let backend = MockBackend::new();
let backend = backend
    // At most 64 inputs per request.
    .batch_chunked(64)
    // 600 requests a minute, in bursts of up to 20.
    .rate_limited(Rate::per_minute(600).with_burst(20))
    // At most 8 requests in flight.
    .concurrency_limited(8);
# }
```

- **`ConcurrencyLimited`** caps the calls in flight at once. A burst waits
  for a free slot instead of opening a connection per call.
- **`RateLimited`** caps calls over time to a `Rate`, with a token bucket: it
  refills at a steady rate, and an idle period banks calls for a burst. A
  `Rate` bursts up to its count by default; `with_burst` changes that.
- **`BatchChunked`** splits a batch larger than the provider accepts into
  batches that fit, sent one after another, and joins their responses in
  order.

The three limit different things and combine in any order. A batch takes
one concurrency slot, since it is one round trip, but one rate token per
request in it, since providers count a batch's items against their quota.
