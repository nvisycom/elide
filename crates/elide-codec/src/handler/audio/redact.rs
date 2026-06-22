//! Shared audio redaction over a decoded interleaved sample buffer.
//!
//! Both handlers decode to interleaved samples, redact, then re-encode.
//! The redaction itself is format- and sample-type-agnostic: given a
//! [`TimeSpan`], it silences or removes the corresponding span of the
//! buffer. Callers must apply a batch in descending time order so a
//! [`Removed`] span doesn't shift the indices
//! of spans not yet applied.
//!
//! [`Removed`]: AudioReplacement::Removed

use elide_core::modality::audio::AudioReplacement;
use elide_core::primitive::TimeSpan;

/// Microseconds per second.
const MICROS_PER_SECOND: u128 = 1_000_000;

/// Frame-aligned interleaved-sample index for a microsecond offset.
///
/// `micros * sample_rate * channels / 1_000_000`, computed with the
/// multiplication first (in `u128`) so the per-sample resolution isn't
/// lost to integer truncation, then clamped to `buffer_len`. Every channel
/// of a frame maps to the same instant, so the result silences or removes
/// whole frames together.
fn sample_index(micros: u64, sample_rate: u32, channels: u16, buffer_len: usize) -> usize {
    let frames = micros as u128 * sample_rate as u128 * channels as u128 / MICROS_PER_SECOND;
    (frames as usize).min(buffer_len)
}

/// Apply `replacement` to the `span` of `samples`.
///
/// `Silenced` zeroes the span in place (duration preserved); `Removed`
/// drains it (the clip shortens). A zero-length or out-of-range span is a
/// no-op.
pub(super) fn apply<S: Default + Clone>(
    samples: &mut Vec<S>,
    span: TimeSpan,
    replacement: &AudioReplacement,
    sample_rate: u32,
    channels: u16,
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
        AudioReplacement::Removed => {
            samples.drain(start..end);
        }
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
        apply(&mut samples, span, &AudioReplacement::Silenced, 1_000, 1);
        assert_eq!(samples, vec![7, 7, 0, 0, 0, 7, 7, 7, 7, 7]);
    }

    #[test]
    fn remove_drains_span() {
        let mut samples = vec![1i16, 2, 3, 4, 5, 6];
        let span = TimeSpan::from_millis(2, 4);
        apply(&mut samples, span, &AudioReplacement::Removed, 1_000, 1);
        assert_eq!(samples, vec![1, 2, 5, 6]);
    }

    #[test]
    fn frame_aligned_for_stereo() {
        // 1000 Hz stereo: 2 samples per ms. Silencing ms 1..2 hits one
        // frame = samples [2, 3].
        let mut samples = vec![9i16; 8];
        let span = TimeSpan::from_millis(1, 2);
        apply(&mut samples, span, &AudioReplacement::Silenced, 1_000, 2);
        assert_eq!(samples, vec![9, 9, 0, 0, 9, 9, 9, 9]);
    }
}
