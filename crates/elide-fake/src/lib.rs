#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod catalog;
mod generator;
mod identity;
mod locale;
mod operator;
mod synth;

pub use self::generator::FakeGenerator;
pub use self::operator::Fake;
