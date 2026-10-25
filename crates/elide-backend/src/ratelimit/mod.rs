//! Backend decorators that bound how a provider is called.
//!
//! [`ConcurrencyLimited`] caps *simultaneous* calls with a semaphore — how many
//! run at the provider at once. [`RateLimited`] caps calls *over time* with a
//! token bucket — a requests-per-interval quota. [`BatchChunked`] caps the
//! *size* of one batch — splitting an oversized `call_batch` into
//! provider-sized sub-batches. They are different limits and compose: wrap a
//! backend in any combination to bound its open connections, request rate, and
//! batch size at the one place every call passes through.

mod batch;
mod concurrency;
mod rate;

pub use self::batch::BatchChunked;
pub use self::concurrency::ConcurrencyLimited;
pub use self::rate::RateLimited;
