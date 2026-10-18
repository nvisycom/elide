//! Backend decorators that bound how a provider is called.
//!
//! [`ConcurrencyLimited`] caps *simultaneous* calls with a semaphore — how many
//! run at the provider at once. [`RateLimited`] caps calls *over time* with a
//! token bucket — a requests-per-interval quota. They are different limits and
//! compose: wrap a backend in both to bound its open connections and its request
//! rate at the one place every call passes through.

mod concurrency;
mod rate;

pub use self::concurrency::ConcurrencyLimited;
pub use self::rate::RateLimited;
