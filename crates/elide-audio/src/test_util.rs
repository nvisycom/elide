//! Small in-memory audio fixtures, behind the `test-util` feature.
//!
//! A downstream crate exercising an audio handler needs a real WAV or MP3 clip
//! without re-implementing the encoders or taking a direct dependency on
//! `hound` or `mp3lame-encoder`. These helpers build such fixtures in this
//! crate's vocabulary, reusing the same engine encoders the redaction path uses.

use bytes::Bytes;

/// A `secs`-long 8 kHz mono 16-bit WAV ramp.
///
/// The smallest fixture a WAV handler can open, decode, and re-encode: a real
/// RIFF container whose samples ascend so a redacted span is visibly zeroed
/// against a non-zero background.
#[cfg(feature = "wav")]
#[must_use]
pub fn wav_ramp(secs: u32) -> Bytes {
    use std::io::Cursor;

    use hound::{SampleFormat, WavSpec, WavWriter};

    let spec = WavSpec {
        channels: 1,
        sample_rate: 8_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut buf = Cursor::new(Vec::new());
    {
        let mut writer = WavWriter::new(&mut buf, spec).expect("wav writer");
        for i in 0..(8_000 * secs) as i32 {
            writer
                .write_sample((i % 1000) as i16)
                .expect("write sample");
        }
        writer.finalize().expect("finalize wav");
    }
    Bytes::from(buf.into_inner())
}

/// A `secs`-long 16 kHz mono MP3 tone at 64 kbps.
///
/// A real MP3 a handler can open and re-encode, built with the same LAME path
/// the redaction round-trip uses, so the fixture and the engine agree on
/// container framing.
#[cfg(feature = "mp3")]
#[must_use]
pub fn mp3_tone(secs: u32) -> Bytes {
    let samples: Vec<f32> = (0..(16_000 * secs))
        .map(|i| ((i as f32) * 0.05).sin() * 0.5)
        .collect();
    let encoded = crate::engine::mp3::encode_from_pcm(&samples, 16_000, 1, 64_000)
        .expect("encode mp3 fixture");
    Bytes::from(encoded)
}
