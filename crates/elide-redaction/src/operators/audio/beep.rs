//! [`Beep`]: replace the matched audio range with a tone.

use elide_audio::modality::{Audio, AudioData, AudioReplacement, Waveform};
use elide_core::entity::Entity;
use elide_core::primitive::ComponentId;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Error, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The broadcast censor-beep frequency, in hertz.
const DEFAULT_HZ: f32 = 1000.0;
/// Peak amplitude, well below full scale so the tone is audible but not
/// painful and never clips.
const DEFAULT_AMPLITUDE: f32 = 0.5;

/// Replace the matched audio range with a tone (the broadcast "bleep").
///
/// More obvious to a listener than [`Silence`] that something was removed,
/// the timeline does not shift, but the redacted span is audibly marked.
/// Defaults to a 1 kHz sine at half amplitude, the broadcast convention.
///
/// [`Silence`]: crate::operators::Silence
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "BeepConfig"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    ///
    /// # Errors
    ///
    /// [`Configuration`](elide_core::ErrorKind::Configuration) if `hz` is not
    /// a finite, positive frequency.
    pub fn new(hz: f32) -> Result<Self> {
        if !hz.is_finite() || hz <= 0.0 {
            return Err(Error::configuration(format!(
                "beep frequency {hz} must be a finite, positive number of hertz"
            )));
        }
        Ok(Self {
            hz,
            ..Self::default()
        })
    }

    /// Set the peak amplitude, in `0.0..=1.0` of full scale.
    ///
    /// # Errors
    ///
    /// [`Configuration`](elide_core::ErrorKind::Configuration) if `amplitude`
    /// is outside `0.0..=1.0`.
    pub fn with_amplitude(mut self, amplitude: f32) -> Result<Self> {
        if !(0.0..=1.0).contains(&amplitude) {
            return Err(Error::configuration(format!(
                "beep amplitude {amplitude} must be within 0.0..=1.0"
            )));
        }
        self.amplitude = amplitude;
        Ok(self)
    }

    /// Set the tone shape.
    #[must_use]
    pub fn with_waveform(mut self, waveform: Waveform) -> Self {
        self.waveform = waveform;
        self
    }
}

/// The deserialized form of a [`Beep`], validated through [`Beep::new`] and
/// [`Beep::with_amplitude`].
#[cfg(feature = "serde")]
#[derive(Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct BeepConfig {
    /// Tone frequency, in hertz.
    hz: f32,
    /// Peak amplitude, in `0.0..=1.0` of full scale.
    amplitude: f32,
    /// Tone shape.
    waveform: Waveform,
}

#[cfg(feature = "serde")]
impl TryFrom<BeepConfig> for Beep {
    type Error = Error;

    fn try_from(config: BeepConfig) -> Result<Self> {
        Ok(Self::new(config.hz)?
            .with_amplitude(config.amplitude)?
            .with_waveform(config.waveform))
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

#[async_trait::async_trait]
impl Operator<Audio> for Beep {
    fn id(&self) -> ComponentId {
        ComponentId::new("beep", "1.0.0")
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
    use elide_audio::modality::AudioLocation;
    use elide_audio::primitive::TimeSpan;
    use elide_core::entity::audit::{AuditEvent, PatternRule};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::primitive::Confidence;

    use super::*;

    fn audio_entity() -> Entity<Audio> {
        let location = AudioLocation::new(TimeSpan::from_millis(0, 100));
        let event = AuditEvent::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternRule::default(),
        );
        Entity::new(LabelRef::new("PERSON"), location).with_event(event)
    }

    #[tokio::test]
    async fn default_is_a_1khz_sine() {
        let out = Beep::default()
            .anonymize(&audio_entity(), &elide_audio::fixtures::blank_audio_data())
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
            .unwrap()
            .with_amplitude(0.25)
            .unwrap()
            .with_waveform(Waveform::Square)
            .anonymize(&audio_entity(), &elide_audio::fixtures::blank_audio_data())
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

    /// A frequency or amplitude outside its range is rejected.
    #[test]
    fn out_of_range_settings_are_rejected() {
        for hz in [0.0, -440.0, f32::NAN, f32::INFINITY] {
            assert!(Beep::new(hz).is_err(), "hz {hz}");
        }
        for amplitude in [-0.1, 1.5, f32::NAN] {
            assert!(
                Beep::default().with_amplitude(amplitude).is_err(),
                "amplitude {amplitude}"
            );
        }
    }
}
