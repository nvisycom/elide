//! [`AudioReplacement`]: what an audio operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;

/// Shape of a synthesized tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Waveform {
    /// Pure sine. The broadcast censor-beep convention: audible but smooth,
    /// with no harmonics to alias on resampling.
    Sine,
    /// Square wave. Harsher and richer in harmonics — the "retro" bleep.
    Square,
}

/// What an audio operator produces to hide an entity: an acoustic
/// treatment applied to the entity's time range.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AudioReplacement {
    /// Replace the range with silence, preserving its duration so the
    /// timeline does not shift.
    Silenced,
    /// Overlay a synthesized tone (the broadcast "bleep"), preserving the
    /// range's duration. More obviously redacted than silence.
    Tone {
        /// Tone frequency in hertz (1000 Hz is the broadcast standard).
        hz: f32,
        /// Peak amplitude in `0.0..=1.0` of full scale.
        amplitude: f32,
        /// Tone shape.
        waveform: Waveform,
    },
    /// Cut the range out entirely, shortening the stream.
    Removed,
    /// Leave the range untouched. Lets a policy keep a tagged range while
    /// redacting everything else.
    Unchanged,
}

impl ModalityReplacement for AudioReplacement {}
