//! [`Erase`]: remove the matched entity entirely, in any modality.

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityReplacement};
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Removes an entity entirely, leaving nothing in its place.
///
/// The strongest treatment: no trace of the value, its shape, or its extent
/// remains, text drops the characters, audio cuts the interval, an image
/// clears the region. One [`Erase`] serves every modality, a downstream one
/// included, through its [`ModalityReplacement::removed`].
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "erase", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Erase;

#[async_trait::async_trait]
impl<M: Modality> Operator<M> for Erase {
    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<M>,
        _data: &M::Data,
    ) -> Result<Option<M::Replacement>> {
        Ok(Some(M::Replacement::removed()))
    }
}
