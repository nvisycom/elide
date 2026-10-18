#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod metrics;
#[cfg(feature = "ratelimit")]
#[cfg_attr(docsrs, doc(cfg(feature = "ratelimit")))]
pub mod ratelimit;
