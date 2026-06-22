//! [`Silence`]: replace the matched audio interval with silence.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::audio::{Audio, AudioData, AudioReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Silence the matched audio interval, preserving its duration.
///
/// The audio-native in-place treatment: the span is zeroed, so the clip's
/// length and the timing of everything after it are unchanged — only the
/// sensitive audio is gone. Contrast [`Erase`], which cuts the interval out
/// and shortens the clip.
///
/// [`Erase`]: super::Erase
#[derive(Debug, Clone, Copy, Default)]
pub struct Silence;

impl Operator<Audio> for Silence {
    fn id(&self) -> OperatorId {
        OperatorId::new("silence", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The duration of the redacted span stays observable; its content
        // does not.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Audio>,
        _data: &AudioData,
    ) -> Result<AudioReplacement> {
        Ok(AudioReplacement::Silenced)
    }
}
