//! Shared audio redaction over a decoded interleaved sample buffer.
//!
//! Both handlers decode to interleaved samples, redact, then re-encode.
//! The redaction itself is format- and sample-type-agnostic: given a time
//! range in milliseconds, it silences or removes the corresponding span
//! of the buffer. Callers must apply a batch in descending time order so
//! a [`Removed`](AudioReplacement::Removed) span doesn't shift the
//! indices of spans not yet applied.

use elide_core::modality::audio::AudioReplacement;

/// Convert a `[start_ms, end_ms)` time range to interleaved-sample
/// indices, clamped to `buffer_len`.
///
/// Indices are frame-aligned: the per-millisecond sample count is
/// `sample_rate * channels / 1000`, so every channel of a frame is
/// silenced or removed together.
fn sample_range(
    start_ms: u64,
    end_ms: u64,
    sample_rate: u32,
    channels: u16,
    buffer_len: usize,
) -> (usize, usize) {
    let per_ms = sample_rate as u64 * channels as u64 / 1_000;
    let start = (start_ms * per_ms) as usize;
    let end = (end_ms * per_ms) as usize;
    (start.min(buffer_len), end.min(buffer_len))
}

/// Apply `replacement` to the `[start_ms, end_ms)` span of `samples`.
///
/// `Silenced` zeroes the span in place (duration preserved); `Removed`
/// drains it (the clip shortens). A zero-length or out-of-range span is a
/// no-op.
pub(super) fn apply<S: Default + Clone>(
    samples: &mut Vec<S>,
    start_ms: u64,
    end_ms: u64,
    replacement: &AudioReplacement,
    sample_rate: u32,
    channels: u16,
) {
    let (start, end) = sample_range(start_ms, end_ms, sample_rate, channels, samples.len());
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
        apply(&mut samples, 2, 5, &AudioReplacement::Silenced, 1_000, 1);
        assert_eq!(samples, vec![7, 7, 0, 0, 0, 7, 7, 7, 7, 7]);
    }

    #[test]
    fn remove_drains_span() {
        let mut samples = vec![1i16, 2, 3, 4, 5, 6];
        apply(&mut samples, 2, 4, &AudioReplacement::Removed, 1_000, 1);
        assert_eq!(samples, vec![1, 2, 5, 6]);
    }

    #[test]
    fn frame_aligned_for_stereo() {
        // 1000 Hz stereo: 2 samples per ms. Silencing ms 1..2 hits one
        // frame = samples [2, 3].
        let mut samples = vec![9i16; 8];
        apply(&mut samples, 1, 2, &AudioReplacement::Silenced, 1_000, 2);
        assert_eq!(samples, vec![9, 9, 0, 0, 9, 9, 9, 9]);
    }
}
