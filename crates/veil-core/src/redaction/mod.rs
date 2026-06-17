//! Redaction: the operator contracts for hiding detected entities.
//!
//! An operator reads an [`Entity`] and the [`Data`] under it and
//! *computes* a [`Replacement`] — mask, replace, hash, encrypt, blur, …
//! — without mutating anything; applying the replacement back into the
//! document is the codec's job. The forward direction is [`Operator`];
//! the optional reverse is [`ReversibleOperator`]. Every operator is an
//! [`Operator`]; only reversible ones (encrypt → decrypt) additionally
//! implement [`ReversibleOperator`].
//!
//! This module defines the operator contracts and the [`Redactions`]
//! batch they feed into. Concrete operators and the label→operator
//! registry that selects them live in `veil-toolkit`.
//!
//! [`Entity`]: crate::entity::Entity
//! [`Data`]: crate::modality::Modality::Data
//! [`Replacement`]: crate::modality::Modality::Replacement

mod operator;
mod operator_id;
mod redactions;
mod reversible;

pub use self::operator::{LeakProfile, Operator};
pub use self::operator_id::OperatorId;
pub use self::redactions::Redactions;
pub use self::reversible::ReversibleOperator;
