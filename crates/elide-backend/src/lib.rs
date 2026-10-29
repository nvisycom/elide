#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod limit;
pub mod metrics;
pub mod routing;

mod ext;

pub use self::ext::BackendExt;
