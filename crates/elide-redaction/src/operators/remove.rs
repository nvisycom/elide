//! [`Remove`]: remove the matched entity entirely, in any modality.

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityReplacement};
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Removes an entity entirely, leaving nothing in its place.
///
/// The strongest treatment: no trace of the value, its shape, or its extent
/// remains, text drops the characters and audio cuts the interval. An image
/// can't lose its pixels, so its region is obscured in place, its extent
/// still observable. One [`Remove`] serves every modality, a downstream one
/// included, through its [`ModalityReplacement::removed`], and leaks what
/// [`ModalityReplacement::removed_leak`] says.
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "remove", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Remove;

#[async_trait::async_trait]
impl<M: Modality> Operator<M> for Remove {
    fn leak_profile(&self) -> LeakProfile {
        M::Replacement::removed_leak()
    }

    async fn anonymize(
        &self,
        _entity: &Entity<M>,
        _data: &M::Data,
    ) -> Result<Option<M::Replacement>> {
        Ok(Some(M::Replacement::removed()))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::Text;
    use elide_core::redaction::{LeakProfile, Operator};
    #[cfg(feature = "image")]
    use elide_image::modality::Image;

    use super::Remove;

    /// A removal leaks what its modality's does: nothing from text, an
    /// image region's extent.
    #[test]
    fn a_removal_leaks_what_its_modality_leaves() {
        assert_eq!(
            Operator::<Text>::leak_profile(&Remove),
            LeakProfile::Irrecoverable
        );
        #[cfg(feature = "image")]
        assert_eq!(
            Operator::<Image>::leak_profile(&Remove),
            LeakProfile::Partial
        );
    }
}
