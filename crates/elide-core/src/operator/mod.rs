//! Operator contracts: how a detected entity is hidden.
//!
//! An operator reads an [`Entity`] and the [`Data`] under it and
//! *computes* a [`Replacement`] (mask, replace, hash, encrypt, blur, …)
//! without mutating anything; applying the replacement back into the
//! document is the codec's job. The forward direction is [`Operator`];
//! the optional reverse is [`ReversibleOperator`]. Every operator is an
//! [`Operator`]; only reversible ones (encrypt → decrypt) additionally
//! implement [`ReversibleOperator`].
//!
//! This module defines the operator contracts, the [`Redactions`] batch
//! they feed into, and the [`Vault`] reversible operators store recovery
//! data in. The redaction engine — the concrete operators and the
//! label→operator registry that selects them — lives in `elide`.
//!
//! [`Entity`]: crate::entity::Entity
//! [`Data`]: crate::modality::Modality::Data
//! [`Replacement`]: crate::modality::Modality::Replacement

mod contract;
mod operator_id;
mod redactions;
mod reversible;
mod vault;

pub use self::contract::{LeakProfile, Operator};
pub use self::operator_id::OperatorId;
pub use self::redactions::Redactions;
pub use self::reversible::ReversibleOperator;
pub use self::vault::Vault;
