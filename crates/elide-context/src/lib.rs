#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod enhancer;
mod io;
mod matching;
mod rule;

pub use self::enhancer::{Context, Enhancer};
pub use self::io::{ContextEnhanced, Token, Tokens};
pub use self::matching::{KeywordMatcher, LemmaMatcher, SubstringMatcher};
pub use self::rule::{BoostRule, DEFAULT_BOOST, DEFAULT_PREFIX_WORDS, DEFAULT_SUFFIX_WORDS};
