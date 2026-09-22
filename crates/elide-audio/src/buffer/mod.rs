//! [`AudioBuffer`]: an audio clip opened once, then read, redacted, and
//! re-encoded, the single entry point into this crate.

use bytes::Bytes;
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};

use crate::modality::{Audio, AudioFormat, AudioLocation, AudioReplacement};
use crate::primitive::TimeSpan;

/// An audio clip, the single entry point into the crate.
///
/// Unlike a decoded raster image, an `AudioBuffer` retains the **encoded**
/// bytes and its format, and decodes to samples only on [`encode`](Self::encode).
/// This is inherent to audio: WAV must re-encode at its original sample format
/// and bit depth, and MP3 derives its re-encode bitrate from the original byte
/// length, so both need the source bytes at encode time. Redactions accumulate
/// on the clip and apply in one decode → mutate → re-encode pass when
/// [`encode`](Self::encode) runs; a clip with no staged redaction encodes to
/// its source bytes untouched.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    source: Bytes,
    format: AudioFormat,
    /// Accumulated redactions; applied together on encode.
    redactions: Redactions<Audio>,
}

impl AudioBuffer {
    /// Open `bytes`: detect the format and retain the encoded clip.
    ///
    /// Detection is try-parse: each enabled format validates the bytes in
    /// turn (WAV first, then MP3), and the first that accepts them wins. The
    /// one place format is determined; nothing downstream sniffs the bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if the bytes are not a clip any enabled
    /// format can read, or [`ErrorKind::CapabilityUnavailable`] if no audio
    /// format is enabled in this build.
    pub fn open(bytes: &[u8]) -> Result<Self> {
        let source = Bytes::copy_from_slice(bytes);

        #[cfg(feature = "wav")]
        if crate::engine::wav::validate(&source).is_ok() {
            return Ok(Self::wrap(source, AudioFormat::Wav));
        }

        #[cfg(feature = "mp3")]
        if let Ok(channels) = crate::engine::mp3::probe_channels(&source) {
            // LAME encodes only mono and stereo; a >2-channel clip would have
            // to be downmixed, which would edit the unredacted audio, so it is
            // rejected here rather than silently altered on encode.
            if channels > 2 {
                return Err(Error::new(
                    ErrorKind::MalformedInput,
                    format!("MP3 has {channels} channels; only mono and stereo are supported"),
                ));
            }
            return Ok(Self::wrap(source, AudioFormat::Mp3));
        }

        #[cfg(feature = "_internal")]
        {
            Err(Error::new(
                ErrorKind::MalformedInput,
                "not a clip any enabled audio format can read",
            ))
        }
        #[cfg(not(feature = "_internal"))]
        {
            let _ = source;
            Err(Error::new(
                ErrorKind::CapabilityUnavailable,
                "no audio format is enabled in this build",
            ))
        }
    }

    /// Wrap already-classified clip bytes with an empty redaction batch.
    #[cfg(feature = "_internal")]
    fn wrap(source: Bytes, format: AudioFormat) -> Self {
        Self {
            source,
            format,
            redactions: Redactions::new(),
        }
    }

    /// The format the clip was opened as, and re-encodes to.
    #[must_use]
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// The clip's duration in milliseconds.
    ///
    /// Read from the source container: the accumulated redactions do not
    /// affect it until [`encode`](Self::encode) produces new bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if the duration cannot be determined from
    /// the container.
    pub fn duration_ms(&self) -> Result<u64> {
        match self.format {
            #[cfg(feature = "wav")]
            AudioFormat::Wav => crate::engine::duration::probe_duration_ms(&self.source, "wav"),
            #[cfg(feature = "mp3")]
            AudioFormat::Mp3 => crate::engine::mp3::duration_ms(&self.source),
        }
    }

    /// Stage a batch of redactions, appended to any already accumulated.
    ///
    /// They apply together on the next [`encode`](Self::encode); staging does
    /// not decode the clip.
    pub fn redact_batch(&mut self, redactions: Redactions<Audio>) {
        for (location, replacement) in redactions {
            self.redactions.push(location, replacement);
        }
    }

    /// Stage a single time-span redaction, a convenience over
    /// [`redact_batch`](Self::redact_batch).
    pub fn redact(&mut self, span: TimeSpan, replacement: &AudioReplacement) {
        self.redactions.push(AudioLocation::new(span), *replacement);
    }

    /// Re-encode the clip with the staged redactions applied, returning the
    /// container bytes.
    ///
    /// With no staged redaction the source bytes are returned untouched (a
    /// cheap clone); otherwise the clip is decoded, every redaction applied,
    /// and the result re-encoded in the source format.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if decoding fails, or
    /// [`ErrorKind::Processing`] if re-encoding fails.
    pub fn encode(&self) -> Result<Bytes> {
        if self.redactions.is_empty() {
            return Ok(self.source.clone());
        }
        match self.format {
            #[cfg(feature = "wav")]
            AudioFormat::Wav => crate::engine::wav::redact_all(&self.source, &self.redactions),
            #[cfg(feature = "mp3")]
            AudioFormat::Mp3 => crate::engine::mp3::redact_all(&self.source, &self.redactions),
        }
    }
}

#[cfg(all(test, feature = "wav"))]
mod tests {
    use std::io::Cursor;

    use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

    use super::*;
    use crate::modality::{AudioLocation, AudioReplacement};

    /// A 1-second 8000 Hz mono i16 WAV ramp, encoded to bytes.
    fn ramp_wav() -> Bytes {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = WavWriter::new(&mut buf, spec).unwrap();
            for i in 0..8_000i32 {
                w.write_sample((i % 1000) as i16).unwrap();
            }
            w.finalize().unwrap();
        }
        Bytes::from(buf.into_inner())
    }

    #[test]
    fn open_detects_wav_and_reports_duration() {
        let clip = AudioBuffer::open(&ramp_wav()).expect("open");
        assert_eq!(clip.format(), AudioFormat::Wav);
        assert_eq!(clip.duration_ms().expect("duration"), 1_000);
    }

    #[test]
    fn open_rejects_garbage() {
        let err = AudioBuffer::open(b"not audio at all").expect_err("garbage rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    #[test]
    fn empty_batch_round_trips_source_bytes() {
        let bytes = ramp_wav();
        let clip = AudioBuffer::open(&bytes).expect("open");
        assert_eq!(clip.encode().expect("encode"), bytes);
    }

    #[test]
    fn silence_zeroes_a_span_and_preserves_length() {
        let mut clip = AudioBuffer::open(&ramp_wav()).expect("open");
        let mut batch: Redactions<Audio> = Redactions::new();
        batch.push(
            AudioLocation::from_millis(100, 200),
            AudioReplacement::Silenced,
        );
        clip.redact_batch(batch);

        let out = clip.encode().expect("encode");
        let mut reader = WavReader::new(Cursor::new(out)).unwrap();
        let samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(samples.len(), 8_000);
        // 8000 Hz mono: ms 100..200 -> samples 800..1600.
        assert!(samples[800..1600].iter().all(|&s| s == 0));
        assert!(samples[0..800].iter().any(|&s| s != 0));
    }

    #[test]
    fn remove_shortens_the_clip() {
        let mut clip = AudioBuffer::open(&ramp_wav()).expect("open");
        clip.redact(TimeSpan::from_millis(0, 500), &AudioReplacement::Removed);

        let out = clip.encode().expect("encode");
        let mut reader = WavReader::new(Cursor::new(out)).unwrap();
        let count = reader.samples::<i16>().count();
        // Removed the first 500ms (4000 samples) of an 8000-sample clip.
        assert_eq!(count, 4_000);
    }
}

#[cfg(all(test, feature = "mp3"))]
mod mp3_tests {
    use bytes::Bytes;
    use elide_core::redaction::Redactions;

    use super::{AudioBuffer, AudioFormat};
    use crate::engine::mp3::{decode_to_pcm, encode_from_pcm};
    use crate::modality::{Audio, AudioLocation, AudioReplacement};

    /// ~0.5s of mono 16 kHz tone, encoded to MP3 bytes.
    fn tone_mp3() -> Bytes {
        let samples: Vec<f32> = (0..8_000)
            .map(|i| ((i as f32) * 0.05).sin() * 0.5)
            .collect();
        Bytes::from(encode_from_pcm(&samples, 16_000, 1, 64_000).unwrap())
    }

    #[test]
    fn open_detects_mp3_and_reports_a_duration() {
        let clip = AudioBuffer::open(&tone_mp3()).expect("open");
        assert_eq!(clip.format(), AudioFormat::Mp3);
        assert!(clip.duration_ms().expect("duration") > 0);
    }

    #[test]
    fn silence_redaction_reencodes_to_valid_mp3() {
        let mut clip = AudioBuffer::open(&tone_mp3()).expect("open");
        let mut batch: Redactions<Audio> = Redactions::new();
        batch.push(
            AudioLocation::from_millis(100, 200),
            AudioReplacement::Silenced,
        );
        clip.redact_batch(batch);

        // The re-encoded clip still decodes, and the silenced span reads as
        // near-zero energy.
        let out = clip.encode().expect("encode");
        let decoded = decode_to_pcm(&out).unwrap();
        assert_eq!(decoded.channels, 1);
        let start = (decoded.sample_rate as usize) * 100 / 1_000;
        let end = (decoded.sample_rate as usize) * 200 / 1_000;
        let span =
            &decoded.samples[start.min(decoded.samples.len())..end.min(decoded.samples.len())];
        let peak = span.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(
            peak < 0.05,
            "silenced span should be near-zero, peak={peak}"
        );
    }
}
