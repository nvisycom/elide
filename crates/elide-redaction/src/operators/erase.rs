//! [`Erase`]: remove the matched entity entirely, in any modality.

#[cfg(feature = "audio")]
use elide_audio::modality::{Audio, AudioData, AudioReplacement};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::text::TextReplacement;
use elide_core::primitive::ComponentId;
use elide_core::redaction::{LeakProfile, Operator};
#[cfg(feature = "image")]
use elide_image::modality::{Image, ImageData, ImageReplacement};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::operators::TextValued;

/// Removes an entity entirely, leaving nothing in its place.
///
/// The strongest treatment: no trace of the value, its shape, or its extent
/// remains, text drops the characters, audio cuts the interval, an image
/// clears the region. One [`Erase`] serves every medium, with a per-modality
/// [`Operator`] impl that maps to that modality's "removed" replacement.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Erase;

impl Erase {
    /// Identity shared by every modality's impl.
    fn id() -> ComponentId {
        ComponentId::new("erase", "1.0.0")
    }
}

#[async_trait::async_trait]
impl<M: TextValued> Operator<M> for Erase {
    fn id(&self) -> ComponentId {
        Erase::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(&self, _entity: &Entity<M>, _data: &M::Data) -> Result<M::Replacement> {
        // A table cell loses its text, not its row.
        Ok(M::replacement(TextReplacement::Removed))
    }
}

#[cfg(feature = "audio")]
#[async_trait::async_trait]
impl Operator<Audio> for Erase {
    fn id(&self) -> ComponentId {
        Erase::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Audio>,
        _data: &AudioData,
    ) -> Result<AudioReplacement> {
        Ok(AudioReplacement::Removed)
    }
}

#[cfg(feature = "image")]
#[async_trait::async_trait]
impl Operator<Image> for Erase {
    fn id(&self) -> ComponentId {
        Erase::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &ImageData,
    ) -> Result<ImageReplacement> {
        Ok(ImageReplacement::Removed)
    }
}
