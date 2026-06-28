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
