#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod backend;
pub mod sink;

#[doc(inline)]
pub use self::backend::{Metered, ModelUsage, Outcome, Usage, UsageCollector, UsageSink};
#[cfg(feature = "metrics")]
#[doc(inline)]
pub use self::sink::MetricsSink;
#[doc(inline)]
pub use self::sink::Tee;
#[cfg(feature = "tracing")]
#[doc(inline)]
pub use self::sink::TracingSink;
