//! [`AudioReplacement`]: what an audio operator produces.

use elide_core::modality::ModalityReplacement;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The shape of the tone an audio operator synthesizes over a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Waveform {
    /// Pure sine. The broadcast censor-beep convention: audible but smooth,
    /// with no harmonics to alias on resampling.
    Sine,
    /// Square wave. Harsher and richer in harmonics, the "retro" bleep.
    Square,
}

/// What an audio operator produces to hide an entity: a sound treatment.
///
/// The treatment applies to the entity's time range.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AudioReplacement {
    /// The range is silenced, its duration kept so the timeline does not
    /// shift.
    Silenced,
    /// A synthesized tone (the broadcast "bleep") is played over the range,
    /// its duration kept. More obviously redacted than silence.
    Beeped {
        /// Tone frequency in hertz (1000 Hz is the broadcast standard).
        hz: f32,
        /// Peak amplitude in `0.0..=1.0` of full scale.
        amplitude: f32,
        /// Tone shape.
        waveform: Waveform,
    },
    /// The range is cut out entirely, shortening the stream.
    Removed,
}

impl ModalityReplacement for AudioReplacement {
    fn removed() -> Self {
        Self::Removed
    }

    fn covered() -> Self {
        Self::Silenced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interval_is_covered_by_silence() {
        assert_eq!(AudioReplacement::covered(), AudioReplacement::Silenced);
    }
}
