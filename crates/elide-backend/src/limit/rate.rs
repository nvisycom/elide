//! [`RateLimited`]: a [`Backend`] decorator that caps requests *over time*.

use std::time::Duration;

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// A token bucket: `capacity` tokens that refill at a steady rate, protected by
/// an async mutex so the token accounting is serialized across callers.
///
/// Tokens are tracked as a float and refilled lazily — on each acquire, the time
/// since the last refill is converted to tokens at the fill rate and added (up to
/// `capacity`). A full bucket absorbs a burst; once drained, acquires pace out at
/// the fill rate.
struct Bucket {
    /// Tokens available now; fractional between whole-token boundaries.
    tokens: f64,
    /// Maximum tokens the bucket holds — the largest burst it absorbs.
    capacity: f64,
    /// Tokens added per second.
    fill_per_sec: f64,
    /// When `tokens` was last refilled.
    updated: Instant,
}

impl Bucket {
    /// Refill from elapsed time, then take one token, returning how long to wait
    /// first if none is available yet.
    fn take(&mut self, now: Instant) -> Option<Duration> {
        let elapsed = now.duration_since(self.updated).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.fill_per_sec).min(self.capacity);
        self.updated = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            None
        } else {
            // Wait for the bucket to earn the fraction of a token it still needs.
            let deficit = 1.0 - self.tokens;
            Some(Duration::from_secs_f64(deficit / self.fill_per_sec))
        }
    }
}

/// A request rate: a number of calls per interval, with an optional burst.
///
/// The burst is how many calls an idle limiter lets through at once; it
/// defaults to the rate's count, a smooth rate with no extra burst. Raise it
/// with [`with_burst`](Self::with_burst) to let an idle period bank calls for a
/// spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    count: u32,
    interval: Duration,
    burst: u32,
}

impl Rate {
    /// `count` calls per `interval`.
    ///
    /// # Panics
    ///
    /// Panics if `count` or `interval` is zero: neither describes a rate that
    /// ever admits a call.
    #[must_use]
    pub fn new(count: u32, interval: Duration) -> Self {
        assert!(count > 0, "rate count must be non-zero");
        assert!(!interval.is_zero(), "rate interval must be non-zero");
        Self {
            count,
            interval,
            burst: count,
        }
    }

    /// `count` calls per second.
    ///
    /// # Panics
    ///
    /// Panics if `count` is zero.
    #[must_use]
    pub fn per_second(count: u32) -> Self {
        Self::new(count, Duration::from_secs(1))
    }

    /// `count` calls per minute.
    ///
    /// # Panics
    ///
    /// Panics if `count` is zero.
    #[must_use]
    pub fn per_minute(count: u32) -> Self {
        Self::new(count, Duration::from_secs(60))
    }

    /// Let up to `burst` calls through at once after an idle period.
    ///
    /// # Panics
    ///
    /// Panics if `burst` is zero: a limiter that holds no call never admits one.
    #[must_use]
    pub fn with_burst(mut self, burst: u32) -> Self {
        assert!(burst > 0, "rate burst must be non-zero");
        self.burst = burst;
        self
    }

    /// Calls admitted per [`interval`](Self::interval).
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The interval the [`count`](Self::count) is measured over.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Calls admitted at once after an idle period.
    pub fn burst(&self) -> u32 {
        self.burst
    }
}

/// Wraps a [`Backend`] and admits at most a fixed number of its calls per
/// interval, leaving the request and response untouched.
///
/// A token bucket meters the [`Rate`]: the bucket holds up to the burst in
/// tokens and refills the count of them every interval, so a full bucket lets a
/// burst through at once and a drained one paces callers out at the steady fill
/// rate.
/// Each call takes one token, waiting when the bucket is empty;
/// [`call_batch`](Backend::call_batch) takes one *per request* — a provider
/// meters a batch against its quota per item, and the default `call_batch` is one
/// `call` per item anyway.
///
/// This caps requests *over time* — a provider's requests-per-interval quota —
/// which is different from bounding *simultaneous* calls. For that, wrap with
/// [`ConcurrencyLimited`]; the two compose, one capping the rate and the other
/// the open connections. Because it is itself a `Backend` (same request/response,
/// same provenance), it drops in anywhere the inner backend goes, and composes
/// with [`Metered`](crate::metrics::Metered).
///
/// [`ConcurrencyLimited`]: crate::limit::ConcurrencyLimited
pub struct RateLimited<B> {
    inner: B,
    bucket: Mutex<Bucket>,
}

impl<B> RateLimited<B> {
    /// Admit `inner`'s calls at `rate`.
    #[must_use]
    pub fn new(inner: B, rate: Rate) -> Self {
        let capacity = f64::from(rate.burst);
        let fill_per_sec = f64::from(rate.count) / rate.interval.as_secs_f64();
        Self {
            inner,
            bucket: Mutex::new(Bucket {
                // Start full: an idle limiter admits an initial burst immediately.
                tokens: capacity,
                capacity,
                fill_per_sec,
                updated: Instant::now(),
            }),
        }
    }

    /// The wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }

    /// Acquire one token, sleeping until the bucket can afford it.
    ///
    /// The bucket mutex is held across the wait so tokens are handed out in
    /// arrival order; it is released before the provider call runs, so calls run
    /// concurrently once admitted.
    async fn acquire(&self) {
        loop {
            let wait = {
                let mut bucket = self.bucket.lock().await;
                match bucket.take(Instant::now()) {
                    None => return,
                    Some(wait) => wait,
                }
            };
            tokio::time::sleep(wait).await;
        }
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
        self.acquire().await;
        self.inner.call(request).await
    }

    async fn call_batch(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>> {
        // One token per request: a batch consumes a provider's per-interval quota
        // per item, whether the inner backend sends it as one batched request or
        // the default `call_batch` loops `call` per item. (This differs from
        // `ConcurrencyLimited`, where a batch is a single connection — there the
        // limit is open connections, here it is quota load.)
        for _ in &requests {
            self.acquire().await;
        }
        self.inner.call_batch(requests).await
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use elide_core::backend::{BackendRequest, BackendResponse};

    use super::*;

    struct Req<'a>(PhantomData<&'a ()>);
    impl BackendRequest for Req<'_> {}

    #[derive(PartialEq, Eq, Debug)]
    struct Resp(usize);
    impl BackendResponse for Resp {}

    /// A backend that counts how many calls reached it, so a test can assert how
    /// many the limiter admitted in a window of time.
    struct Counting {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Backend for Counting {
        type Request<'a> = Req<'a>;
        type Response = Resp;

        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "counting".into(),
                ..ModelEvent::default()
            }
        }

        async fn call(&self, _request: Req<'_>) -> Result<Resp> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Resp(n))
        }
    }

    /// The limiter is transparent: a call returns the inner backend's response.
    #[tokio::test]
    async fn passes_the_call_through() {
        let backend = RateLimited::new(
            Counting {
                calls: AtomicUsize::new(0),
            },
            Rate::per_second(10).with_burst(10),
        );
        let response = backend.call(Req(PhantomData)).await.unwrap();
        assert_eq!(response, Resp(1), "one call reached the inner backend");
    }

    /// A full bucket admits a burst up to its capacity with no wait.
    #[tokio::test(start_paused = true)]
    async fn admits_a_burst_up_to_capacity() {
        let backend = RateLimited::new(
            Counting {
                calls: AtomicUsize::new(0),
            },
            // One token per second, but a bucket of 5 — so 5 go through at once.
            Rate::per_second(1).with_burst(5),
        );

        for _ in 0..5 {
            backend.call(Req(PhantomData)).await.unwrap();
        }
        assert_eq!(
            backend.inner().calls.load(Ordering::SeqCst),
            5,
            "the full bucket let the whole burst through"
        );
    }

    /// Once the bucket drains, further calls pace out at the fill rate. With the
    /// clock paused, the sixth call blocks until time is advanced past the refill
    /// interval.
    #[tokio::test(start_paused = true)]
    async fn paces_calls_after_the_burst() {
        let backend = Arc::new(RateLimited::new(
            Counting {
                calls: AtomicUsize::new(0),
            },
            Rate::per_second(1).with_burst(5),
        ));

        // Drain the 5-token bucket.
        for _ in 0..5 {
            backend.call(Req(PhantomData)).await.unwrap();
        }

        // The sixth call must wait for a token to refill.
        let task = {
            let backend = backend.clone();
            tokio::spawn(async move { backend.call(Req(PhantomData)).await.unwrap() })
        };
        tokio::task::yield_now().await;
        assert_eq!(
            backend.inner().calls.load(Ordering::SeqCst),
            5,
            "the sixth call is still waiting on an empty bucket"
        );

        // One second refills one token; the sixth call proceeds.
        tokio::time::advance(Duration::from_secs(1)).await;
        task.await.unwrap();
        assert_eq!(
            backend.inner().calls.load(Ordering::SeqCst),
            6,
            "the refilled token admitted the waiting call"
        );
    }

    /// A batch charges one token per request, not one per batch: a 3-request
    /// batch against a 2-token bucket blocks until a third token refills.
    #[tokio::test(start_paused = true)]
    async fn a_batch_charges_one_token_per_request() {
        let backend = Arc::new(RateLimited::new(
            Counting {
                calls: AtomicUsize::new(0),
            },
            Rate::per_second(1).with_burst(2),
        ));

        // The bucket holds 2 tokens; a batch of 3 needs a third.
        let task = {
            let backend = backend.clone();
            tokio::spawn(async move {
                let requests = vec![Req(PhantomData), Req(PhantomData), Req(PhantomData)];
                backend.call_batch(requests).await.unwrap()
            })
        };
        tokio::task::yield_now().await;
        assert_eq!(
            backend.inner().calls.load(Ordering::SeqCst),
            0,
            "the batch is still waiting on its third token — no request reached the backend"
        );

        // One second refills the third token; the batch runs.
        tokio::time::advance(Duration::from_secs(1)).await;
        let responses = task.await.unwrap();
        assert_eq!(responses.len(), 3);
        assert_eq!(
            backend.inner().calls.load(Ordering::SeqCst),
            3,
            "all three requests reached the backend once the tokens were charged"
        );
    }
}
