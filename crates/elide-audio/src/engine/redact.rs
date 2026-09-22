//! Shared audio redaction over a decoded interleaved sample buffer.
//!
//! Both formats decode to interleaved samples, redact, then re-encode.
//! The redaction itself is format- and sample-type-agnostic: given a
//! [`TimeSpan`], it silences or removes the corresponding span of the
//! buffer. Callers must apply a batch in descending time order so a
//! [`Removed`] span doesn't shift the indices
//! of spans not yet applied.
//!
//! [`Removed`]: AudioReplacement::Removed

use std::f32::consts::TAU;

use crate::modality::{AudioReplacement, Waveform};
use crate::primitive::TimeSpan;

/// Microseconds per second.
const MICROS_PER_SECOND: u128 = 1_000_000;

/// A sample type a synthesized tone can be written into.
///
/// Maps a normalized amplitude in `-1.0..=1.0` onto the concrete sample
/// representation, so tone synthesis stays format-agnostic.
pub(crate) trait ToneSample {
    /// Convert a normalized `-1.0..=1.0` amplitude to this sample type,
    /// scaled so that `1.0` maps to `full_scale`.
    ///
    /// `full_scale` is the container's peak magnitude, which is not always the
    /// Rust type's `MAX`: a 24-bit WAV decodes into `i32` but a written sample
    /// must stay within `2^23 - 1`, so the caller passes that peak, not
    /// `i32::MAX`. For float samples `full_scale` is `1.0`.
    fn from_unit(value: f32, full_scale: f32) -> Self;
}

/// Implement [`ToneSample`] for a signed-integer sample type by scaling the
/// normalized amplitude to the caller-supplied full-scale peak.
macro_rules! int_tone_sample {
    ($($ty:ty),+) => {$(
        impl ToneSample for $ty {
            fn from_unit(value: f32, full_scale: f32) -> Self {
                (value.clamp(-1.0, 1.0) * full_scale) as $ty
            }
        }
    )+};
}

int_tone_sample!(i8, i16, i32);

impl ToneSample for f32 {
    fn from_unit(value: f32, _full_scale: f32) -> Self {
        value.clamp(-1.0, 1.0)
    }
}

/// Frame-aligned interleaved-sample index for a microsecond offset.
///
/// `micros * sample_rate * channels / 1_000_000`, computed with the
/// multiplication first (in `u128`) so the per-sample resolution isn't
/// lost to integer truncation, then rounded down to a whole frame and
/// clamped to `buffer_len`. Aligning to the channel stride keeps every
/// channel of a frame mapped to the same instant, so a span silences or
/// removes whole frames together: an index landing mid-frame would drain an
/// odd sample count and swap the channels for the rest of the clip.
fn sample_index(micros: u64, sample_rate: u32, channels: u16, buffer_len: usize) -> usize {
    let stride = channels.max(1) as usize;
    let raw = micros as u128 * sample_rate as u128 * channels as u128 / MICROS_PER_SECOND;
    let aligned = (raw as usize) / stride * stride;
    aligned.min(buffer_len)
}

/// Apply `replacement` to the `span` of `samples`.
///
/// `Silenced` zeroes the span in place; `Tone` overlays a synthesized tone
/// (both preserve duration); `Removed` drains it (the clip shortens). A
/// zero-length or out-of-range span is a no-op.
///
/// `full_scale` is the peak sample magnitude a synthesized `Tone` scales to
/// (see [`ToneSample::from_unit`]); it is unused for the other replacements.
pub(crate) fn apply<S: Default + Clone + ToneSample>(
    samples: &mut Vec<S>,
    span: TimeSpan,
    replacement: &AudioReplacement,
    sample_rate: u32,
    channels: u16,
    full_scale: f32,
) {
    let start = sample_index(span.start_micros(), sample_rate, channels, samples.len());
    let end = sample_index(span.end_micros(), sample_rate, channels, samples.len());
    if start >= end {
        return;
    }
    match replacement {
        AudioReplacement::Silenced => {
            for s in &mut samples[start..end] {
                *s = S::default();
            }
        }
        AudioReplacement::Tone {
            hz,
            amplitude,
            waveform,
        } => tone(
            &mut samples[start..end],
            start,
            ToneSpec {
                hz: *hz,
                amplitude: *amplitude,
                waveform: *waveform,
                sample_rate,
                channels,
                full_scale,
            },
        ),
        AudioReplacement::Removed => {
            samples.drain(start..end);
        }
        AudioReplacement::Unchanged => {}
    }
}

/// The inputs to tone synthesis, grouped so [`tone`] stays a two-argument
/// call over the span it writes.
struct ToneSpec {
    hz: f32,
    amplitude: f32,
    waveform: Waveform,
    sample_rate: u32,
    channels: u16,
    /// Peak sample magnitude `1.0` maps to (see [`ToneSample::from_unit`]).
    full_scale: f32,
}

/// Overwrite `span` with a synthesized tone.
///
/// `frame_offset` is the interleaved index of the span's first sample, used
/// so phase is continuous from the clip's start (no click at the boundary).
/// All channels of a frame get the same value; the same instant maps to the
/// same phase.
fn tone<S: ToneSample>(span: &mut [S], frame_offset: usize, spec: ToneSpec) {
    let channels = spec.channels.max(1) as usize;
    let amplitude = spec.amplitude.clamp(0.0, 1.0);
    let rate = spec.sample_rate.max(1) as f32;
    for (i, sample) in span.iter_mut().enumerate() {
        let frame = (frame_offset + i) / channels;
        let phase = TAU * spec.hz * frame as f32 / rate;
        let wave = match spec.waveform {
            Waveform::Sine => phase.sin(),
            Waveform::Square => {
                if phase.sin() >= 0.0 {
                    1.0
                } else {
                    -1.0
                }
            }
        };
        *sample = S::from_unit(amplitude * wave, spec.full_scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_zeroes_in_place() {
        // 1000 Hz mono: 1 sample per ms.
        let mut samples = vec![7i16; 10];
        let span = TimeSpan::from_millis(2, 5);
        apply(
            &mut samples,
            span,
            &AudioReplacement::Silenced,
            1_000,
            1,
            0.0,
        );
        assert_eq!(samples, vec![7, 7, 0, 0, 0, 7, 7, 7, 7, 7]);
    }

    #[test]
    fn remove_drains_span() {
        let mut samples = vec![1i16, 2, 3, 4, 5, 6];
        let span = TimeSpan::from_millis(2, 4);
        apply(
            &mut samples,
            span,
            &AudioReplacement::Removed,
            1_000,
            1,
            0.0,
        );
        assert_eq!(samples, vec![1, 2, 5, 6]);
    }

    #[test]
    fn frame_aligned_for_stereo() {
        // 1000 Hz stereo: 2 samples per ms. Silencing ms 1..2 hits one
        // frame = samples [2, 3].
        let mut samples = vec![9i16; 8];
        let span = TimeSpan::from_millis(1, 2);
        apply(
            &mut samples,
            span,
            &AudioReplacement::Silenced,
            1_000,
            2,
            0.0,
        );
        assert_eq!(samples, vec![9, 9, 0, 0, 9, 9, 9, 9]);
    }

    #[test]
    fn remove_drains_whole_frames_for_stereo() {
        // 44100 Hz stereo. An unaligned span (its raw interleaved index is
        // odd) must still drain an even sample count, so the surviving frames
        // keep their L/R order rather than swapping for the rest of the clip.
        let mut samples: Vec<i16> = (0..8).collect();
        // 111 micros -> raw index 44100*2*111/1e6 = 9 (odd) -> aligned to 8.
        let span = TimeSpan::new(111, 200);
        apply(
            &mut samples,
            span,
            &AudioReplacement::Removed,
            44_100,
            2,
            0.0,
        );
        // An even number of samples was drained; the remainder stays paired.
        assert_eq!(samples.len() % 2, 0);
    }

    #[test]
    fn tone_overwrites_span_and_leaves_the_rest() {
        // 8 kHz mono. A tone over ms 2..5 overwrites those samples and
        // leaves everything outside untouched.
        let mut samples = vec![100i16; 8_000];
        let span = TimeSpan::from_millis(2, 5);
        let tone = AudioReplacement::Tone {
            hz: 1_000.0,
            amplitude: 0.5,
            waveform: Waveform::Sine,
        };
        apply(&mut samples, span, &tone, 8_000, 1, i16::MAX as f32);

        // Outside the span is unchanged.
        assert!(samples[..16].iter().all(|&s| s == 100));
        assert!(samples[40..].iter().all(|&s| s == 100));
        // Inside the span the tone wrote bounded, non-constant samples.
        let inside = &samples[16..40];
        assert!(inside.iter().any(|&s| s != 100));
        assert!(
            inside
                .iter()
                .all(|&s| (s as f32).abs() <= 0.5 * i16::MAX as f32 + 1.0)
        );
    }

    #[test]
    fn square_tone_is_two_valued() {
        let mut samples = vec![0i16; 800];
        let span = TimeSpan::from_millis(0, 100);
        let tone = AudioReplacement::Tone {
            hz: 1_000.0,
            amplitude: 0.5,
            waveform: Waveform::Square,
        };
        apply(&mut samples, span, &tone, 8_000, 1, i16::MAX as f32);

        let amp = (0.5 * i16::MAX as f32) as i16;
        assert!(samples.iter().all(|&s| s == amp || s == -amp));
        assert!(samples.contains(&amp));
        assert!(samples.contains(&-amp));
    }
}
