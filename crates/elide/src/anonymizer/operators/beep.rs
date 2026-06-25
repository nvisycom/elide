//! [`Beep`]: overlay a tone over the matched audio range.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::audio::{Audio, AudioData, AudioReplacement, Waveform};
use elide_core::operator::{LeakProfile, Operator, OperatorId};

/// The broadcast censor-beep frequency, in hertz.
const DEFAULT_HZ: f32 = 1000.0;
/// Peak amplitude, well below full scale so the tone is audible but not
/// painful and never clips.
const DEFAULT_AMPLITUDE: f32 = 0.5;

/// Overlay a tone (the broadcast "bleep") over the matched audio range,
/// preserving its duration.
///
/// More obvious to a listener than [`Silence`] that something was removed —
/// the timeline does not shift, but the redacted span is audibly marked.
/// Defaults to a 1 kHz sine at half amplitude, the broadcast convention.
///
/// [`Silence`]: super::Silence
#[derive(Debug, Clone, Copy)]
pub struct Beep {
    /// Tone frequency, in hertz.
    hz: f32,
    /// Peak amplitude, in `0.0..=1.0` of full scale.
    amplitude: f32,
    /// Tone shape.
    waveform: Waveform,
}

impl Beep {
    /// A beep at the given frequency (hz), default amplitude and sine shape.
    pub fn new(hz: f32) -> Self {
        Self {
            hz,
            ..Self::default()
        }
    }

    /// Set the peak amplitude, in `0.0..=1.0` of full scale.
    #[must_use]
    pub fn with_amplitude(mut self, amplitude: f32) -> Self {
        self.amplitude = amplitude;
        self
    }

    /// Set the tone shape.
    #[must_use]
    pub fn with_waveform(mut self, waveform: Waveform) -> Self {
        self.waveform = waveform;
        self
    }
}

impl Default for Beep {
    fn default() -> Self {
        Self {
            hz: DEFAULT_HZ,
            amplitude: DEFAULT_AMPLITUDE,
            waveform: Waveform::Sine,
        }
    }
}

impl Operator<Audio> for Beep {
    fn id(&self) -> OperatorId {
        OperatorId::new("beep", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // Like silence, the duration of the redacted span stays observable;
        // its content does not.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Audio>,
        _data: &AudioData,
    ) -> Result<AudioReplacement> {
        Ok(AudioReplacement::Tone {
            hz: self.hz,
            amplitude: self.amplitude,
            waveform: self.waveform,
        })
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::audio::{AudioData, AudioLocation};
    use elide_core::primitive::{Confidence, TimeSpan};

    use super::*;

    fn audio_entity() -> Entity<Audio> {
        let location = AudioLocation::new(TimeSpan::from_millis(0, 100));
        let event = Event::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternEvent::default(),
        );
        Entity::new(
            LabelRef::new("PERSON"),
            location,
            Confidence::MAX,
            Provenance::new(event),
        )
    }

    #[tokio::test]
    async fn default_is_a_1khz_sine() {
        let out = Beep::default()
            .anonymize(&audio_entity(), &AudioData::new(Vec::<u8>::new()))
            .await
            .unwrap();
        assert_eq!(
            out,
            AudioReplacement::Tone {
                hz: 1000.0,
                amplitude: 0.5,
                waveform: Waveform::Sine,
            }
        );
    }

    #[tokio::test]
    async fn builders_set_frequency_amplitude_and_shape() {
        let out = Beep::new(440.0)
            .with_amplitude(0.25)
            .with_waveform(Waveform::Square)
            .anonymize(&audio_entity(), &AudioData::new(Vec::<u8>::new()))
            .await
            .unwrap();
        assert_eq!(
            out,
            AudioReplacement::Tone {
                hz: 440.0,
                amplitude: 0.25,
                waveform: Waveform::Square,
            }
        );
    }
}
