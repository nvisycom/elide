//! Backend usage: the [`Metered`] wrapper that meters a
//! [`Backend`](elide_core::backend::Backend), the [`Usage`] record it produces,
//! and the [`UsageSink`] it reports to (with the in-memory [`UsageCollector`]).

mod metered;
mod usage;

pub use self::metered::Metered;
pub use self::usage::{ModelUsage, Outcome, Usage, UsageCollector, UsageSink};
