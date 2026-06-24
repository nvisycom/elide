#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod generator;
mod locale;
mod operator;

pub use self::operator::Fake;
