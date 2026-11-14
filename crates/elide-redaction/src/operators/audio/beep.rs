//! [`Beep`]: replace the matched audio range with a tone.

use elide_audio::modality::{Audio, AudioData, AudioReplacement, Waveform};
use elide_core::entity::Entity;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Error, Result};
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
#[derive(Debug, Clone, Copy, Component)]
#[component(name = "beep", version = "1.0.0")]
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
    /// A value outside the range is clamped into it, and NaN falls back to the
    /// default amplitude. Amplitude sets only how loud the tone is: the
    /// matched audio is replaced whatever it is, so no value weakens the
    /// redaction.
    #[must_use]
    pub fn with_amplitude(mut self, amplitude: f32) -> Self {
        self.amplitude = if amplitude.is_nan() {
            DEFAULT_AMPLITUDE
        } else {
            amplitude.clamp(0.0, 1.0)
        };
        self
    }

    /// Set the tone shape.
    #[must_use]
    pub fn with_waveform(mut self, waveform: Waveform) -> Self {
        self.waveform = waveform;
        self
    }
}

/// The deserialized form of a [`Beep`], validated through [`Beep::new`], its
/// amplitude clamped by [`Beep::with_amplitude`].
#[cfg(feature = "serde")]
#[derive(Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct BeepConfig {
    /// Tone frequency, in hertz.
    hz: f32,
    /// Peak amplitude, in `0.0..=1.0` of full scale; clamped into it.
    amplitude: f32,
    /// Tone shape.
    waveform: Waveform,
}

#[cfg(feature = "serde")]
impl TryFrom<BeepConfig> for Beep {
    type Error = Error;

    fn try_from(config: BeepConfig) -> Result<Self> {
        Ok(Self::new(config.hz)?
            .with_amplitude(config.amplitude)
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
    fn leak_profile(&self) -> LeakProfile {
        // Like silence, the duration of the redacted span stays observable;
        // its content does not.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Audio>,
        _data: &AudioData,
    ) -> Result<Option<AudioReplacement>> {
        Ok(Some(AudioReplacement::Tone {
            hz: self.hz,
            amplitude: self.amplitude,
            waveform: self.waveform,
        }))
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
            Some(AudioReplacement::Tone {
                hz: 1000.0,
                amplitude: 0.5,
                waveform: Waveform::Sine,
            })
        );
    }

    #[tokio::test]
    async fn builders_set_frequency_amplitude_and_shape() {
        let out = Beep::new(440.0)
            .unwrap()
            .with_amplitude(0.25)
            .with_waveform(Waveform::Square)
            .anonymize(&audio_entity(), &elide_audio::fixtures::blank_audio_data())
            .await
            .unwrap();
        assert_eq!(
            out,
            Some(AudioReplacement::Tone {
                hz: 440.0,
                amplitude: 0.25,
                waveform: Waveform::Square,
            })
        );
    }

    /// A frequency that is not finite and positive is rejected.
    #[test]
    fn invalid_frequencies_are_rejected() {
        for hz in [0.0, -440.0, f32::NAN, f32::INFINITY] {
            assert!(Beep::new(hz).is_err(), "hz {hz}");
        }
    }

    /// An amplitude outside `0.0..=1.0` is clamped into it; NaN falls back to
    /// the default.
    #[test]
    fn out_of_range_amplitudes_are_clamped() {
        for (amplitude, expected) in [(-0.1, 0.0), (1.5, 1.0), (0.3, 0.3), (f32::NAN, 0.5)] {
            assert_eq!(
                Beep::default().with_amplitude(amplitude).amplitude,
                expected,
                "amplitude {amplitude}"
            );
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserializing_validates_the_frequency_and_clamps_the_amplitude() {
        assert!(
            serde_json::from_str::<Beep>(r#"{"hz":0.0,"amplitude":0.5,"waveform":"sine"}"#)
                .is_err()
        );
        let beep: Beep =
            serde_json::from_str(r#"{"hz":440.0,"amplitude":1.5,"waveform":"sine"}"#).unwrap();
        assert_eq!(beep.amplitude, 1.0);
    }
}
