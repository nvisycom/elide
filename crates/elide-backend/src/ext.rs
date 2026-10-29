//! [`BackendExt`]: wrap a backend in a decorator by method, in the order the
//! decorators apply.

use elide_core::backend::Backend;

use crate::limit::BatchChunked;
#[cfg(feature = "ratelimit")]
use crate::limit::{ConcurrencyLimited, Rate, RateLimited};
use crate::metrics::{Metered, UsageSink};
use crate::routing::{FallbackRouted, LanguageRouted};

/// Wraps a [`Backend`] in this crate's decorators by method.
///
/// Each method is the decorator's constructor with the backend as its first
/// argument, so a stack of decorators reads in the order they wrap, innermost
/// first, instead of inside out:
///
/// ```ignore
/// let backend = gliner
///     .metered(usage.clone())
///     .rate_limited(Rate::per_minute(600))
///     .concurrency_limited(8);
/// ```
///
/// Implemented for every backend.
pub trait BackendExt: Backend + Sized {
    /// Report each call's usage to `sink`; see [`Metered`].
    fn metered(self, sink: impl UsageSink + 'static) -> Metered<Self> {
        Metered::new(self, sink)
    }

    /// Send at most `max_batch_size` requests per batch; see [`BatchChunked`].
    ///
    /// # Panics
    ///
    /// Panics if `max_batch_size` is zero.
    fn batch_chunked(self, max_batch_size: usize) -> BatchChunked<Self> {
        BatchChunked::new(self, max_batch_size)
    }

    /// Run at most `max_concurrent` calls at once; see [`ConcurrencyLimited`].
    ///
    /// # Panics
    ///
    /// Panics if `max_concurrent` is zero.
    #[cfg(feature = "ratelimit")]
    #[cfg_attr(docsrs, doc(cfg(feature = "ratelimit")))]
    fn concurrency_limited(self, max_concurrent: usize) -> ConcurrencyLimited<Self> {
        ConcurrencyLimited::new(self, max_concurrent)
    }

    /// Admit calls at `rate`; see [`RateLimited`].
    #[cfg(feature = "ratelimit")]
    #[cfg_attr(docsrs, doc(cfg(feature = "ratelimit")))]
    fn rate_limited(self, rate: Rate) -> RateLimited<Self> {
        RateLimited::new(self, rate)
    }

    /// Route requests by language, with this backend as the fallback; add
    /// routes with [`LanguageRouted::route`].
    fn language_routed(self) -> LanguageRouted<Self> {
        LanguageRouted::new(self)
    }

    /// Route a call this backend fails to `fallback`; see [`FallbackRouted`].
    fn fallback_routed<F>(self, fallback: F) -> FallbackRouted<Self, F> {
        FallbackRouted::new(self, fallback)
    }
}

impl<B: Backend> BackendExt for B {}
