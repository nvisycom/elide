#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod anonymizer;
mod deanonymizer;

// The core operator contract the engines are generic over. The shipped
// operators themselves live in `elide-operator`; this crate only *selects and
// applies* them, so it names the trait, not the concrete types.
#[doc(inline)]
pub use elide_core::redaction::{
    LeakProfile, Operator, OperatorId, Redactions, ReversibleOperator,
};

pub use self::anonymizer::{Anonymizer, MatchContext, Rule};
pub use self::deanonymizer::Deanonymizer;
