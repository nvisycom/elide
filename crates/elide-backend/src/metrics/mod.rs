//! Usage accounting: the [`Metered`] decorator and where its records go.
//!
//! [`Metered`] records each backend call's [`Usage`]; the [`UsageCollector`]
//! keeps the records, and the other [`UsageSink`]s route them onward.
//!
//! Always compiled — the types pull no extra dependency. The sinks that forward
//! to an external system are feature-gated by the system they target:
//! `TracingSink` (feature `tracing`) and `MetricsSink` (feature `recorder`).

mod metered;
#[cfg(feature = "recorder")]
mod recorder_sink;
mod sink;
#[cfg(feature = "tracing")]
mod tracing_sink;
mod usage;

pub use self::metered::Metered;
#[cfg(feature = "recorder")]
#[cfg_attr(docsrs, doc(cfg(feature = "recorder")))]
pub use self::recorder_sink::MetricsSink;
pub use self::sink::Tee;
#[cfg(feature = "tracing")]
#[cfg_attr(docsrs, doc(cfg(feature = "tracing")))]
pub use self::tracing_sink::TracingSink;
pub use self::usage::{ModelUsage, Outcome, Usage, UsageCollector, UsageSink};
