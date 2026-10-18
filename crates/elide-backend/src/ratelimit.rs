//! [`RateLimited`]: a [`Backend`] decorator that caps concurrent calls.

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;
use tokio::sync::Semaphore;

/// Wraps a [`Backend`] and admits at most a fixed number of its calls at once,
/// leaving the request and response untouched.
///
/// Concurrency against a provider is bounded here, at the one place every call
/// to that provider passes through, rather than at each fan-out site above it (an
/// analyzer running recognizers in parallel, an escalation decorator running
/// subjects in parallel). A caller may spawn any number of concurrent calls; the
/// permit count decides how many actually reach the provider at a time, so a
/// burst waits instead of hitting a rate limit. [`call_batch`](Backend::call_batch)
/// takes a single permit — a batch is one round-trip.
///
/// Because it is itself a `Backend` (same request/response, same provenance), it
/// drops in anywhere the inner backend goes, and composes with
/// [`Metered`](crate::metrics::Metered): wrap the inner with whichever ordering the caller
/// wants to meter (metering the waited call, or the call plus its wait).
pub struct RateLimited<B> {
    inner: B,
    permits: Semaphore,
}

impl<B> RateLimited<B> {
    /// Admit at most `max_concurrent` of `inner`'s calls at once.
    ///
    /// # Panics
    ///
    /// Panics if `max_concurrent` is zero — a zero-permit limiter would block
    /// every call forever.
    #[must_use]
    pub fn new(inner: B, max_concurrent: usize) -> Self {
        assert!(max_concurrent > 0, "max_concurrent must be non-zero");
        Self {
            inner,
            permits: Semaphore::new(max_concurrent),
        }
    }

    /// The wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }
}

#[async_trait::async_trait]
impl<B: Backend> Backend for RateLimited<B> {
    type Request<'a> = B::Request<'a>;
    type Response = B::Response;

    fn provenance(&self) -> ModelEvent {
        self.inner.provenance()
    }

    async fn call(&self, request: Self::Request<'_>) -> Result<Self::Response> {
        // The semaphore is never closed, so acquiring cannot fail. The permit is
        // held for the duration of the call and released when it drops.
        let _permit = self
            .permits
            .acquire()
            .await
            .expect("rate-limit semaphore is never closed");
        self.inner.call(request).await
    }

    async fn call_batch(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>> {
        // One permit for the whole batch: it is a single provider round-trip.
        let _permit = self
            .permits
            .acquire()
            .await
            .expect("rate-limit semaphore is never closed");
        self.inner.call_batch(requests).await
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use elide_core::backend::{BackendRequest, BackendResponse};

    use super::*;

    struct Req<'a>(PhantomData<&'a ()>);
    impl BackendRequest for Req<'_> {}

    #[derive(PartialEq, Eq, Debug)]
    struct Resp(usize);
    impl BackendResponse for Resp {}

    /// A backend that records how many calls are in flight at once, so a test can
    /// assert the limiter's cap. Each call holds for a beat, then returns the
    /// in-flight count it observed.
    struct Tracking {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Backend for Tracking {
        type Request<'a> = Req<'a>;
        type Response = Resp;

        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "tracking".into(),
                ..ModelEvent::default()
            }
        }

        async fn call(&self, _request: Req<'_>) -> Result<Resp> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(Resp(now))
        }
    }

    /// The limiter is transparent: a call returns the inner backend's response.
    #[tokio::test]
    async fn passes_the_call_through() {
        let backend = RateLimited::new(
            Tracking {
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
            },
            4,
        );
        let response = backend.call(Req(PhantomData)).await.unwrap();
        assert_eq!(response, Resp(1), "one call, one in flight");
    }

    /// Ten concurrent calls against a limit of 3 never run more than 3 at once.
    #[tokio::test]
    async fn caps_concurrent_calls() {
        let backend = Arc::new(RateLimited::new(
            Tracking {
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
            },
            3,
        ));

        let tasks: Vec<_> = (0..10)
            .map(|_| {
                let backend = backend.clone();
                tokio::spawn(async move { backend.call(Req(PhantomData)).await.unwrap() })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }

        assert_eq!(
            backend.inner().peak.load(Ordering::SeqCst),
            3,
            "never more than the limit in flight"
        );
    }
}
