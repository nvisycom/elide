//! [`Remove`]: remove the matched entity entirely.

#[cfg(feature = "audio")]
use elide_audio::modality::{Audio, AudioReplacement};
use elide_core::entity::Entity;
use elide_core::modality::Modality;
#[cfg(feature = "metadata")]
use elide_core::modality::metadata::Metadata;
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Removes an entity entirely, leaving nothing in its place.
///
/// The strongest treatment: nothing of the value, its content or its
/// extent, remains. Text drops the characters, a table cell its text, a
/// metadata field its value, and audio cuts the interval. An image can't lose
/// its pixels, so [`Remove`] doesn't apply to one; [`Cover`] hides a region in
/// place. A downstream modality that can remove a value implements
/// [`Operator`] for [`Remove`] itself.
///
/// [`Cover`]: crate::operators::Cover
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "remove", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Remove;

/// `impl Operator<$modality> for Remove`, writing `$removed`.
macro_rules! removes {
    ($modality:ty => $removed:expr) => {
        #[async_trait::async_trait]
        impl Operator<$modality> for Remove {
            fn leak_profile(&self) -> LeakProfile {
                LeakProfile::Irrecoverable
            }

            async fn anonymize(
                &self,
                _entity: &Entity<$modality>,
                _data: &<$modality as Modality>::Data,
            ) -> Result<Option<<$modality as Modality>::Replacement>> {
                Ok(Some($removed))
            }
        }
    };
}

removes!(Text => TextReplacement::Removed);
#[cfg(feature = "metadata")]
removes!(Metadata => TextReplacement::Removed);
#[cfg(feature = "tabular")]
removes!(Tabular => TabularReplacement::CellRedacted(TextReplacement::Removed));
#[cfg(feature = "audio")]
removes!(Audio => AudioReplacement::Removed);

#[cfg(test)]
mod tests {
    use elide_core::entity::Entity;
    use elide_core::modality::text::{Text, TextData, TextReplacement};
    use elide_core::redaction::{LeakProfile, Operator};

    use super::Remove;

    #[tokio::test]
    async fn text_is_removed() {
        let entity = Entity::<Text>::fixture("PERSON", 0..8);
        let out = Remove
            .anonymize(&entity, &TextData::new("Jane Roe"))
            .await
            .unwrap();
        assert_eq!(out, Some(TextReplacement::Removed));
        assert_eq!(
            Operator::<Text>::leak_profile(&Remove),
            LeakProfile::Irrecoverable
        );
    }
}
