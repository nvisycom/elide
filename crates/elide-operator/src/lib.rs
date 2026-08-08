#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod generator;
pub mod operators;
pub mod vault;

// The core operator contract — `Operator`, `ReversibleOperator`, `OperatorId`,
// `LeakProfile`, `Redactions` — re-surfaced here so a caller reaches the trait
// and the shipped operators from one crate.
#[doc(inline)]
pub use elide_core::operator::*;
