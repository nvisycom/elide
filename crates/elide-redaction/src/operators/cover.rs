//! [`Cover`]: hide the matched entity in place, keeping its extent, in any
//! modality.

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityReplacement};
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Hides an entity in place, keeping its extent and marking it.
///
/// The visual redaction: the value is gone, but where it was and how much of
/// it there was stay observable. Text is covered (a box over it in a PDF,
/// one `█` per character in text), audio silenced for the interval, an
/// image region covered in black. One [`Cover`] serves every modality, a
/// downstream one included, through its [`ModalityReplacement::covered`].
/// Contrast [`Remove`], which leaves nothing that marks where the value was.
///
/// [`Remove`]: crate::operators::Remove
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "cover", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Cover;

#[async_trait::async_trait]
impl<M: Modality> Operator<M> for Cover {
    fn leak_profile(&self) -> LeakProfile {
        // The value is gone; its extent stays observable.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<M>,
        _data: &M::Data,
    ) -> Result<Option<M::Replacement>> {
        Ok(Some(M::Replacement::covered()))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::Entity;
    use elide_core::modality::text::{Text, TextData, TextReplacement};
    use elide_core::redaction::Operator;

    use super::Cover;

    #[tokio::test]
    async fn text_is_covered() {
        let entity = Entity::<Text>::fixture("PERSON", (0, 8));
        let out = Cover
            .anonymize(&entity, &TextData::new("Jane Roe"))
            .await
            .unwrap();
        assert_eq!(out, Some(TextReplacement::Covered));
    }
}
