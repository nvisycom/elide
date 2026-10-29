//! [`BatchChunked`]: a [`Backend`] decorator that splits an oversized batch into
//! provider-sized sub-batches.

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;

/// Wraps a [`Backend`] and caps how many requests one
/// [`call_batch`](Backend::call_batch) sends to the provider at a time.
///
/// A provider that accepts batches often caps the batch size (an embedding or
/// NER endpoint that rejects more than, say, 64 inputs per request). The
/// batching above this decorator — a recognizer or enricher that coalesces a
/// whole run's subjects into one `call_batch` — does not know that limit, so it
/// can hand down a batch larger than the provider allows. This decorator splits
/// such a batch into consecutive windows of at most `max_batch_size`, issues one
/// inner `call_batch` per window, and concatenates the responses in request
/// order, so the caller sees exactly the responses a single large batch would
/// have returned.
///
/// Windows are issued **sequentially**: `call_batch` awaits each window before
/// it builds the next, so this reshapes a batch to fit the provider's size
/// limit without adding parallelism. Composing it with
/// [`ConcurrencyLimited`](crate::limit::ConcurrencyLimited) therefore does
/// not run one batch's windows concurrently — it bounds concurrency across
/// *separate* calls; compose with
/// [`RateLimited`](crate::limit::RateLimited) to bound the request rate.
/// Because it is itself a `Backend` (same request/response, same provenance), it
/// drops in anywhere the inner backend goes.
///
/// A single [`call`](Backend::call) passes straight through — it is already one
/// request.
pub struct BatchChunked<B> {
    inner: B,
    max_batch_size: usize,
}

impl<B> BatchChunked<B> {
    /// Send at most `max_batch_size` requests to `inner` per `call_batch`.
    ///
    /// # Panics
    ///
    /// Panics if `max_batch_size` is zero — a zero-size window could never make
    /// progress on a non-empty batch.
    #[must_use]
    pub fn new(inner: B, max_batch_size: usize) -> Self {
        assert!(max_batch_size > 0, "max_batch_size must be non-zero");
        Self {
            inner,
            max_batch_size,
        }
    }

    /// The wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }
}

#[async_trait::async_trait]
impl<B: Backend> Backend for BatchChunked<B> {
    type Request<'a> = B::Request<'a>;
    type Response = B::Response;

    fn provenance(&self) -> ModelEvent {
        self.inner.provenance()
    }

    async fn call(&self, request: Self::Request<'_>) -> Result<Self::Response> {
        self.inner.call(request).await
    }

    async fn call_batch(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>> {
        // A batch within the limit is one round-trip, unchanged.
        if requests.len() <= self.max_batch_size {
            return self.inner.call_batch(requests).await;
        }

        let mut responses = Vec::with_capacity(requests.len());
        // Consume the owned requests through one iterator, taking the next window
        // off its front each round: in order, so the concatenated responses stay
        // request-aligned, and without the quadratic tail-shift a repeated
        // `Vec::drain` would cause on a batch split into many windows.
        let mut requests = requests.into_iter();
        while let Some(first) = requests.next() {
            let mut window = Vec::with_capacity(self.max_batch_size);
            window.push(first);
            window.extend(requests.by_ref().take(self.max_batch_size - 1));
            responses.extend(self.inner.call_batch(window).await?);
        }
        Ok(responses)
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;
    use std::sync::Mutex;

    use elide_core::backend::{BackendRequest, BackendResponse};

    use super::*;

    /// A request carrying an id, so a test can assert response order. The
    /// lifetime matches the `Backend::Request<'a>` GAT even though nothing is
    /// borrowed.
    #[derive(Clone, Copy)]
    struct Req<'a>(usize, PhantomData<&'a ()>);
    impl BackendRequest for Req<'_> {}

    impl Req<'_> {
        fn new(id: usize) -> Self {
            Self(id, PhantomData)
        }
    }

    #[derive(PartialEq, Eq, Debug)]
    struct Resp(usize);
    impl BackendResponse for Resp {}

    /// A backend that records the size of every `call_batch` it receives and
    /// echoes each request's id back, so a test can assert both the split
    /// windows and that responses stay request-aligned.
    struct Recording {
        batch_sizes: Mutex<Vec<usize>>,
    }

    impl Recording {
        fn new() -> Self {
            Self {
                batch_sizes: Mutex::new(Vec::new()),
            }
        }

        fn batch_sizes(&self) -> Vec<usize> {
            self.batch_sizes.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Backend for Recording {
        type Request<'a> = Req<'a>;
        type Response = Resp;

        fn provenance(&self) -> ModelEvent {
            ModelEvent::default()
        }

        async fn call(&self, request: Req<'_>) -> Result<Resp> {
            Ok(Resp(request.0))
        }

        async fn call_batch(&self, requests: Vec<Req<'_>>) -> Result<Vec<Resp>> {
            self.batch_sizes.lock().unwrap().push(requests.len());
            Ok(requests.into_iter().map(|r| Resp(r.0)).collect())
        }
    }

    fn reqs(n: usize) -> Vec<Req<'static>> {
        (0..n).map(Req::new).collect()
    }

    fn resps(n: usize) -> Vec<Resp> {
        (0..n).map(Resp).collect()
    }

    #[tokio::test]
    async fn splits_an_oversized_batch_into_windows_preserving_order() {
        let chunked = BatchChunked::new(Recording::new(), 3);
        let out = chunked.call_batch(reqs(10)).await.unwrap();

        // Responses are request-aligned and complete.
        assert_eq!(out, resps(10));
        // 10 requests / 3 = windows of 3, 3, 3, 1.
        assert_eq!(chunked.inner().batch_sizes(), vec![3, 3, 3, 1]);
    }

    #[tokio::test]
    async fn a_batch_within_the_limit_is_one_round_trip() {
        let chunked = BatchChunked::new(Recording::new(), 5);
        let out = chunked.call_batch(reqs(5)).await.unwrap();
        assert_eq!(out, resps(5));
        assert_eq!(chunked.inner().batch_sizes(), vec![5]);
    }

    #[tokio::test]
    async fn an_empty_batch_makes_no_split() {
        let chunked = BatchChunked::new(Recording::new(), 3);
        let out = chunked.call_batch(Vec::new()).await.unwrap();
        assert!(out.is_empty());
        // len 0 <= limit, so one inner call with an empty vec.
        assert_eq!(chunked.inner().batch_sizes(), vec![0]);
    }

    #[tokio::test]
    #[should_panic(expected = "max_batch_size must be non-zero")]
    async fn zero_limit_panics() {
        let _ = BatchChunked::new(Recording::new(), 0);
    }
}
