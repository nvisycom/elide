//! [`Erase`]: remove the matched entity entirely, in any modality.

#[cfg(feature = "audio")]
use elide_audio::modality::{Audio, AudioData, AudioReplacement};
use elide_core::entity::Entity;
#[cfg(feature = "metadata")]
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataReplacement};
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "image")]
use elide_image::modality::{Image, ImageData, ImageReplacement};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Removes an entity entirely, leaving nothing in its place.
///
/// The strongest treatment: no trace of the value, its shape, or its extent
/// remains, text drops the characters, audio cuts the interval, an image
/// clears the region. One [`Erase`] serves every medium, with a per-modality
/// [`Operator`] impl that maps to that modality's "removed" replacement.
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "erase", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Erase;

#[async_trait::async_trait]
impl Operator<Text> for Erase {
    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(&self, _entity: &Entity<Text>, _data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::Removed)
    }
}

#[cfg(feature = "tabular")]
#[async_trait::async_trait]
impl Operator<Tabular> for Erase {
    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Tabular>,
        _data: &TextData,
    ) -> Result<TabularReplacement> {
        // A table cell loses its text, not its row.
        Ok(TabularReplacement::Cell(TextReplacement::Removed))
    }
}

#[cfg(feature = "metadata")]
#[async_trait::async_trait]
impl Operator<Metadata> for Erase {
    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Metadata>,
        _data: &MetadataData,
    ) -> Result<MetadataReplacement> {
        Ok(MetadataReplacement::Value(TextReplacement::Removed))
    }
}

#[cfg(feature = "audio")]
#[async_trait::async_trait]
impl Operator<Audio> for Erase {
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
