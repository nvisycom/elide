#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod detection;
pub mod enrichment;
pub mod modality;
pub mod recognition;
pub mod redaction;

#[cfg(feature = "codec")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec")))]
pub mod codec;

/// Re-export of [`async_trait`] for implementing the toolkit's async traits.
///
/// The public async traits (`Recognizer`, `Operator`, `Enricher`,
/// `DataReader`, …) are `#[async_trait]`, so an `impl` block must carry the
/// attribute. Use this re-export instead of depending on `async-trait`
/// directly — the version is guaranteed to match:
///
/// ```ignore
/// #[elide::async_trait]
/// impl Recognizer<Text> for MyRecognizer { /* async fn recognize … */ }
/// ```
///
/// [`async_trait`]: async_trait::async_trait
pub use async_trait::async_trait;
#[doc(inline)]
pub use elide_core::{Error, ErrorKind, Result};
#[doc(inline)]
pub use elide_core::{entity, primitive};
// The orchestration engine (`Orchestrator`, `Report`, `Directives`,
// `EntityGroup`) — a small curated set, re-exported flat at the root under the
// `engine` feature. `engine` implies `codec` (the orchestrator decodes through
// it) and serde.
#[cfg(feature = "engine")]
#[cfg_attr(docsrs, doc(cfg(feature = "engine")))]
#[doc(inline)]
pub use elide_engine::{Directives, EntityGroup, Orchestrator, Report};

pub mod prelude;
