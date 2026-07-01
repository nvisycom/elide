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
#[cfg(feature = "codec")]
#[doc(hidden)]
pub use elide_orchestration::EntityGroup;
#[cfg(feature = "codec")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec")))]
#[doc(inline)]
pub use elide_orchestration::{Orchestrator, Report};

pub mod prelude;
