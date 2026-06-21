//! MP3 handler: holds the encoded clip and redacts by decoding to PCM
//! with `symphonia`, mutating the buffer, and re-encoding with LAME.

use bytes::Bytes;
use elide_core::Result;
use elide_core::modality::audio::{Audio, AudioData, AudioLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::mp3_codec::{average_bitrate_bps, decode_to_pcm, duration_ms, encode_from_pcm};
use super::redact;
use crate::content::ContentData;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the MP3 codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.audio.mp3");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Audio, _>(FORMAT_ID.clone(), super::mp3_loader::Mp3Loader)
        .with_extensions(["mp3"])
        .with_content_types(["audio/mpeg"])
}

/// Handler for a loaded MP3 clip. Holds the encoded bytes; PCM decode
/// happens only when a redaction needs it.
#[derive(Debug)]
pub(crate) struct Mp3Handler {
    bytes: Bytes,
    yielded: bool,
}

impl Mp3Handler {
    /// Wrap encoded MP3 bytes; the streaming cursor starts unyielded.
    pub(crate) fn new(bytes: Bytes) -> Self {
        Self {
            bytes,
            yielded: false,
        }
    }
}

impl Handler<Audio> for Mp3Handler {
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
        let total_ms = duration_ms(&self.bytes)?;
        self.yielded = true;
        Ok(Some(Chunk {
            location: AudioLocation::from_millis(0, total_ms),
            data: AudioData::new(self.bytes.clone()),
            hints: Vec::new(),
        }))
    }
}

impl DataReader<Audio> for Mp3Handler {
    async fn read_at(&self, _location: &AudioLocation) -> Result<Option<AudioData>> {
        Ok(Some(AudioData::new(self.bytes.clone())))
    }
}

impl DataWriter<Audio> for Mp3Handler {
    async fn write_at(&mut self, mut redactions: Redactions<Audio>) -> Result<()> {
        if redactions.is_empty() {
            return Ok(());
        }
        let total_ms = duration_ms(&self.bytes)?;
        let bitrate_bps = average_bitrate_bps(self.bytes.len(), total_ms);

        let mut decoded = decode_to_pcm(&self.bytes)?;
        // Apply right-to-left so a `Removed` span doesn't shift the
        // sample indices of spans not yet applied.
        redactions.sort_by_position();
        for (location, replacement) in redactions.iter().rev() {
            redact::apply(
                &mut decoded.samples,
                location.span,
                replacement,
                decoded.sample_rate,
                decoded.channels,
            );
        }

        let reencoded = encode_from_pcm(
            &decoded.samples,
            decoded.sample_rate,
            decoded.channels,
            bitrate_bps,
        )?;
        self.bytes = Bytes::from(reencoded);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::audio::AudioReplacement;

    use super::super::mp3_codec::encode_from_pcm;
    use super::*;

    /// ~0.5s of mono 16 kHz tone, encoded to MP3 bytes.
    fn tone_mp3() -> Bytes {
        let samples: Vec<f32> = (0..8_000)
            .map(|i| ((i as f32) * 0.05).sin() * 0.5)
            .collect();
        Bytes::from(encode_from_pcm(&samples, 16_000, 1, 64_000).unwrap())
    }

    #[tokio::test]
    async fn stream_reports_a_duration() {
        let mut h = Mp3Handler::new(tone_mp3());
        let chunk = h.read_next().await.unwrap().expect("one chunk");
        assert_eq!(chunk.location.span.start_millis(), 0);
        assert!(
            chunk.location.span.end_millis() > 0,
            "duration should be positive"
        );
        assert!(h.read_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn silence_redaction_reencodes_to_valid_mp3() {
        let mut h = Mp3Handler::new(tone_mp3());
        let mut batch: Redactions<Audio> = Redactions::new();
        batch.push(
            AudioLocation::from_millis(100, 200),
            AudioReplacement::Silenced,
        );
        h.write_at(batch).await.unwrap();

        // The re-encoded clip still decodes, and the silenced span reads
        // as near-zero energy.
        let out = h.encode().unwrap();
        let decoded = decode_to_pcm(&out.into_bytes()).unwrap();
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
