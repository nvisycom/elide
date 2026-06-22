//! [`Erase`]: remove the matched entity entirely, in any modality.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

use crate::modality::Erasable;

/// Remove the matched entity entirely.
///
/// The strongest treatment: no trace of the value, its shape, or its extent
/// remains. Modality-agnostic via [`Erasable`] — text drops the characters,
/// audio cuts the interval, an image clears the region — so one operator
/// serves every medium.
#[derive(Debug, Clone, Copy, Default)]
pub struct Erase;

impl<M: Erasable> Operator<M> for Erase {
    fn id(&self) -> OperatorId {
        OperatorId::new("erase", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(&self, _entity: &Entity<M>, _data: &M::Data) -> Result<M::Replacement> {
        Ok(M::erased())
    }
}
