//! Backend decorators that bound how a provider is called.
//!
//! [`BatchChunked`] caps the *size* of one batch, splitting an oversized
//! `call_batch` into provider-sized sub-batches. [`ConcurrencyLimited`] caps
//! *simultaneous* calls with a semaphore: how many run at the provider at once.
//! [`RateLimited`] caps calls *over time* with a token bucket, to a [`Rate`]: a
//! requests-per-interval quota. They are different limits and compose: wrap a
//! backend in any combination to bound its batch size, open connections, and
//! request rate at the one place every call passes through.
//!
//! `BatchChunked` needs no runtime and is always available; the other two wait
//! on Tokio primitives and come with the `ratelimit` feature.

mod batch;
#[cfg(feature = "ratelimit")]
mod concurrency;
#[cfg(feature = "ratelimit")]
mod rate;

pub use self::batch::BatchChunked;
#[cfg(feature = "ratelimit")]
#[cfg_attr(docsrs, doc(cfg(feature = "ratelimit")))]
pub use self::concurrency::ConcurrencyLimited;
#[cfg(feature = "ratelimit")]
#[cfg_attr(docsrs, doc(cfg(feature = "ratelimit")))]
pub use self::rate::{Rate, RateLimited};
