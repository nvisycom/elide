//! [`Erase`]: remove the matched entity entirely, in any modality.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
#[cfg(feature = "audio")]
use elide_core::modality::audio::{Audio, AudioReplacement};
#[cfg(feature = "image")]
use elide_core::modality::image::{Image, ImageReplacement};
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Remove the matched entity entirely.
///
/// The strongest treatment: no trace of the value, its shape, or its extent
/// remains — text drops the characters, audio cuts the interval, an image
/// clears the region. One `Erase` serves every medium, with a per-modality
/// [`Operator`] impl that maps to that modality's "removed" replacement.
#[derive(Debug, Clone, Copy, Default)]
pub struct Erase;

impl Erase {
    /// Identity shared by every modality's impl.
    fn id() -> OperatorId {
        OperatorId::new("erase", "1.0.0")
    }
}

impl Operator<Text> for Erase {
    fn id(&self) -> OperatorId {
        Erase::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Text>,
        _data: &<Text as Modality>::Data,
    ) -> Result<TextReplacement> {
        Ok(TextReplacement::Removed)
    }
}

#[cfg(feature = "tabular")]
impl Operator<Tabular> for Erase {
    fn id(&self) -> OperatorId {
        Erase::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Tabular>,
        _data: &<Tabular as Modality>::Data,
    ) -> Result<TabularReplacement> {
        // Erasing a tabular entity removes the cell's text, not the row.
        Ok(TabularReplacement::Cell(TextReplacement::Removed))
    }
}

#[cfg(feature = "audio")]
impl Operator<Audio> for Erase {
    fn id(&self) -> OperatorId {
        Erase::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Audio>,
        _data: &<Audio as Modality>::Data,
    ) -> Result<AudioReplacement> {
        Ok(AudioReplacement::Removed)
    }
}

#[cfg(feature = "image")]
impl Operator<Image> for Erase {
    fn id(&self) -> OperatorId {
        Erase::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &<Image as Modality>::Data,
    ) -> Result<ImageReplacement> {
        Ok(ImageReplacement::Removed)
    }
}
