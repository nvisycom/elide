//! Redaction: the operator contracts for hiding detected entities.
//!
//! An operator reads an [`Entity`](crate::entity::Entity) and the
//! [`Data`](crate::modality::Modality::Data) under it and *computes* a
//! [`Replacement`](crate::modality::Modality::Replacement) — mask,
//! replace, hash, encrypt, blur, … — without mutating anything; applying
//! the replacement back into the document is the codec's job. The forward
//! direction is [`Anonymizer`]; the optional reverse is [`Deanonymizer`].
//! Every operator is an [`Anonymizer`]; only reversible ones (encrypt →
//! decrypt) additionally implement [`Deanonymizer`].
//!
//! This module defines only the contracts. Concrete operators and the
//! label→operator registry that selects them live in `veil-toolkit`.

mod anonymizer;
mod deanonymizer;
mod operator_id;

pub use self::anonymizer::{Anonymizer, LeakProfile};
pub use self::deanonymizer::Deanonymizer;
pub use self::operator_id::OperatorId;
