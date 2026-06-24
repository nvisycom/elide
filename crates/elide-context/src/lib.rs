#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod enhancer;
mod io;
pub mod matching;
mod recognition;
mod rule;

pub use self::enhancer::{Boost, Context, Enhancer};
pub use self::io::{Token, Tokens};
pub use self::recognition::{DraftEvent, Enhanced, EntityDraft, StreamRecognizer, lift, lift_all};
pub use self::rule::{BoostRule, DEFAULT_BOOST, DEFAULT_PREFIX_WORDS, DEFAULT_SUFFIX_WORDS};
