//! [`Cover`]: hide the matched entity in place, keeping its extent.

#[cfg(feature = "audio")]
use elide_audio::modality::{Audio, AudioReplacement};
use elide_core::entity::Entity;
use elide_core::modality::Modality;
#[cfg(feature = "metadata")]
use elide_core::modality::metadata::Metadata;
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextReplacement};
use elide_core::primitive::Color;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "image")]
use elide_image::modality::{Image, ImageReplacement};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Hides an entity in place, keeping its extent and marking it.
///
/// The visual redaction: the value is gone, but where it was and how much of
/// it there was stay observable. Text, a table cell and a metadata field are
/// covered, one `█` per character where the format writes characters and a
/// box where it draws one; audio is silenced for the interval, and an image
/// region covered. A box is drawn in the cover's [`Color`], black by default.
/// A downstream modality implements [`Operator`] for [`Cover`] itself.
/// Contrast [`Remove`], which leaves nothing that marks where the value was.
///
/// [`Remove`]: crate::operators::Remove
#[derive(Debug, Clone, Copy, Component)]
#[component(name = "cover", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Cover {
    /// The color a box is drawn in.
    color: Color,
}

impl Cover {
    /// A cover whose box is drawn in `color`.
    pub const fn new(color: Color) -> Self {
        Self { color }
    }
}

impl Default for Cover {
    fn default() -> Self {
        Self::new(Color::BLACK)
    }
}

/// `impl Operator<$modality> for Cover`, writing `$covered` with the cover's
/// color bound to `$color`.
macro_rules! covers {
    ($modality:ty, $color:pat => $covered:expr) => {
        #[async_trait::async_trait]
        impl Operator<$modality> for Cover {
            fn leak_profile(&self) -> LeakProfile {
                // The value is gone; its extent stays observable.
                LeakProfile::Partial
            }

            async fn anonymize(
                &self,
                _entity: &Entity<$modality>,
                _data: &<$modality as Modality>::Data,
            ) -> Result<Option<<$modality as Modality>::Replacement>> {
                let $color = self.color;
                Ok(Some($covered))
            }
        }
    };
}

covers!(Text, color => TextReplacement::covered(color));
#[cfg(feature = "metadata")]
covers!(Metadata, color => TextReplacement::covered(color));
#[cfg(feature = "tabular")]
covers!(Tabular, color => TabularReplacement::CellRedacted(TextReplacement::covered(color)));
#[cfg(feature = "audio")]
covers!(Audio, _ => AudioReplacement::Silenced);
#[cfg(feature = "image")]
covers!(Image, color => ImageReplacement::Covered { color });

#[cfg(test)]
mod tests {
    use elide_core::entity::Entity;
    use elide_core::modality::text::{Text, TextData, TextReplacement};
    use elide_core::primitive::Color;
    use elide_core::redaction::Operator;

    use super::Cover;

    #[tokio::test]
    async fn text_is_covered_in_black_by_default() {
        let entity = Entity::<Text>::fixture("PERSON", 0..8);
        let out = Cover::default()
            .anonymize(&entity, &TextData::new("Jane Roe"))
            .await
            .unwrap();
        assert_eq!(out, Some(TextReplacement::covered(Color::BLACK)));
    }

    #[tokio::test]
    async fn a_cover_carries_its_color() {
        let red = Color::rgb(200, 0, 0);
        let entity = Entity::<Text>::fixture("PERSON", 0..8);
        let out = Cover::new(red)
            .anonymize(&entity, &TextData::new("Jane Roe"))
            .await
            .unwrap();
        assert_eq!(out, Some(TextReplacement::covered(red)));
    }
}
