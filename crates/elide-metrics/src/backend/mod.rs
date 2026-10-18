//! Backend decorators: the [`Metered`] wrapper that meters a
//! [`Backend`](elide_core::backend::Backend), the [`RateLimited`] wrapper that
//! caps its concurrent calls, the [`Usage`] record metering produces, and the
//! [`UsageSink`] it reports to (with the in-memory [`UsageCollector`]).

mod metered;
mod rate_limited;
mod usage;

pub use self::metered::Metered;
pub use self::rate_limited::RateLimited;
pub use self::usage::{ModelUsage, Outcome, Usage, UsageCollector, UsageSink};
