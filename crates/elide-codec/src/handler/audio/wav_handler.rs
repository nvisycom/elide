//! WAV handler: holds the encoded clip and redacts by decoding samples
//! with `hound`, mutating the buffer, and re-encoding.

use std::io::Cursor;
use std::result;

use bytes::Bytes;
use elide_core::modality::audio::{Audio, AudioData, AudioLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::operator::Redactions;
use elide_core::{Error, ErrorKind, Result};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use super::duration::probe_duration_ms;
use super::redact;
use super::wav_loader::WavLoader;
use crate::content::ContentData;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the WAV codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.audio.wav");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Audio, _>(FORMAT_ID.clone(), WavLoader)
        .with_extensions(["wav"])
        .with_content_types(["audio/wav", "audio/x-wav"])
}

/// Handler for a loaded WAV clip. Holds the encoded bytes; sample-level
/// decoding happens only when a redaction needs it.
#[derive(Debug)]
pub(crate) struct WavHandler {
    bytes: Bytes,
    yielded: bool,
}

impl WavHandler {
    /// Wrap encoded WAV bytes; the streaming cursor starts unyielded.
    pub(crate) fn new(bytes: Bytes) -> Self {
        Self {
            bytes,
            yielded: false,
        }
    }
}

#[async_trait::async_trait]
impl Handler<Audio> for WavHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::new(self.bytes.clone()))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Audio>>> {
        if self.yielded {
            return Ok(None);
        }
        let duration_ms = probe_duration_ms(&self.bytes, "wav")?;
        self.yielded = true;
        Ok(Some(Chunk {
            location: AudioLocation::from_millis(0, duration_ms),
            data: AudioData::new(self.bytes.clone()),
            hints: Vec::new(),
        }))
    }
}

#[async_trait::async_trait]
impl DataReader<Audio> for WavHandler {
    async fn read_at(&self, _location: &AudioLocation) -> Result<Option<AudioData>> {
        // The whole clip is the addressable unit; a partial time range
        // still resolves to the full audio for downstream extraction.
        Ok(Some(AudioData::new(self.bytes.clone())))
    }
}

#[async_trait::async_trait]
impl DataWriter<Audio> for WavHandler {
    async fn write_at(&mut self, mut redactions: Redactions<Audio>) -> Result<()> {
        if redactions.is_empty() {
            return Ok(());
        }
        // Sort by position so the per-type pass can walk the batch in a
        // deterministic order (and reverse it to apply right-to-left).
        redactions.sort_by_position();
        let spec = wav_spec(&self.bytes)?;
        match (spec.sample_format, spec.bits_per_sample) {
            (SampleFormat::Int, 8) => self.redact_typed::<i8>(spec, &redactions)?,
            (SampleFormat::Int, 16) => self.redact_typed::<i16>(spec, &redactions)?,
            (SampleFormat::Int, 24 | 32) => self.redact_typed::<i32>(spec, &redactions)?,
            (SampleFormat::Float, 32) => self.redact_typed::<f32>(spec, &redactions)?,
            (fmt, bits) => {
                return Err(Error::new(
                    ErrorKind::Validation,
                    format!("unsupported WAV sample format {fmt:?} at {bits} bits"),
                ));
            }
        }
        Ok(())
    }
}

impl WavHandler {
    /// Decode every sample as `S`, apply the batch on the buffer, then
    /// re-encode to `self.bytes`.
    fn redact_typed<S>(&mut self, spec: WavSpec, redactions: &Redactions<Audio>) -> Result<()>
    where
        S: hound::Sample + Default + Clone + redact::ToneSample,
    {
        let mut reader = WavReader::new(Cursor::new(self.bytes.clone()))
            .map_err(|e| Error::new(ErrorKind::Validation, format!("WAV read failed: {e}")))?;
        let mut samples: Vec<S> = reader
            .samples::<S>()
            .collect::<result::Result<_, _>>()
            .map_err(|e| Error::new(ErrorKind::Validation, format!("WAV decode failed: {e}")))?;

        // Walk the position-sorted batch in reverse, applying
        // right-to-left so a `Removed` span doesn't shift the sample
        // indices of spans not yet applied.
        for (location, replacement) in redactions.iter().rev() {
            redact::apply(
                &mut samples,
                location.span,
                replacement,
                spec.sample_rate,
                spec.channels,
            );
        }

        let mut buf = Cursor::new(Vec::new());
        {
            let mut writer = WavWriter::new(&mut buf, spec)
                .map_err(|e| Error::new(ErrorKind::Validation, format!("WAV write failed: {e}")))?;
            for sample in samples {
                writer.write_sample(sample).map_err(|e| {
                    Error::new(ErrorKind::Validation, format!("WAV encode failed: {e}"))
                })?;
            }
            writer.finalize().map_err(|e| {
                Error::new(ErrorKind::Validation, format!("WAV finalize failed: {e}"))
            })?;
        }
        self.bytes = Bytes::from(buf.into_inner());
        Ok(())
    }
}

/// Read the [`WavSpec`] (sample rate, channels, bit depth, format) from
/// the encoded bytes.
fn wav_spec(bytes: &Bytes) -> Result<WavSpec> {
    let reader = WavReader::new(Cursor::new(bytes.clone()))
        .map_err(|e| Error::new(ErrorKind::Validation, format!("not a valid WAV: {e}")))?;
    Ok(reader.spec())
}

#[cfg(test)]
mod tests {
    use elide_core::modality::audio::AudioReplacement;

    use super::*;

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

    #[tokio::test]
    async fn stream_reports_one_second() {
        let mut h = WavHandler::new(ramp_wav());
        let chunk = h.read_next().await.unwrap().expect("one chunk");
        assert_eq!(chunk.location.span.start_millis(), 0);
        assert_eq!(chunk.location.span.end_millis(), 1_000);
        assert!(h.read_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn silence_zeroes_a_span_and_preserves_length() {
        let mut h = WavHandler::new(ramp_wav());
        let mut batch: Redactions<Audio> = Redactions::new();
        batch.push(
            AudioLocation::from_millis(100, 200),
            AudioReplacement::Silenced,
        );
        h.write_at(batch).await.unwrap();

        // Re-read: the clip is still 1s and samples in 100..200ms are zero.
        let out = h.encode().unwrap();
        let mut reader = WavReader::new(Cursor::new(out.into_bytes())).unwrap();
        let samples: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(samples.len(), 8_000);
        // 8000 Hz mono: ms 100..200 -> samples 800..1600.
        assert!(samples[800..1600].iter().all(|&s| s == 0));
        assert!(samples[0..800].iter().any(|&s| s != 0));
    }

    #[tokio::test]
    async fn remove_shortens_the_clip() {
        let mut h = WavHandler::new(ramp_wav());
        let mut batch: Redactions<Audio> = Redactions::new();
        batch.push(
            AudioLocation::from_millis(0, 500),
            AudioReplacement::Removed,
        );
        h.write_at(batch).await.unwrap();

        let out = h.encode().unwrap();
        let mut reader = WavReader::new(Cursor::new(out.into_bytes())).unwrap();
        let count = reader.samples::<i16>().count();
        // Removed the first 500ms (4000 samples) of an 8000-sample clip.
        assert_eq!(count, 4_000);
    }
}
