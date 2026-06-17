//! Redaction: the operator contracts for hiding detected entities.
//!
//! An operator reads an [`Entity`](crate::entity::Entity) and the
//! [`Data`](crate::modality::Modality::Data) under it and *computes* a
//! [`Replacement`](crate::modality::Modality::Replacement) — mask,
//! replace, hash, encrypt, blur, … — without mutating anything; applying
//! the replacement back into the document is the codec's job. The forward
//! direction is [`Operator`]; the optional reverse is
//! [`ReversibleOperator`]. Every operator is an [`Operator`]; only
//! reversible ones (encrypt → decrypt) additionally implement
//! [`ReversibleOperator`].
//!
//! This module defines only the contracts. Concrete operators and the
//! label→operator registry that selects them live in `veil-toolkit`.

mod operator;
mod operator_id;
mod reversible;

pub use self::operator::{LeakProfile, Operator};
pub use self::operator_id::OperatorId;
pub use self::reversible::ReversibleOperator;
