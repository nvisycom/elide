//! [`AudioStream`]: the clip body part — streams the whole clip as one chunk,
//! accumulates redactions, and re-encodes on demand.

use elide_codec::{FormatId, Stream};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Component, Error, Result};

use crate::AudioBuffer;
use crate::modality::{Audio, AudioData, AudioLocation};

/// The clip stream. Holds the clip as an [`AudioBuffer`]; redaction accumulates
/// on it, and the document's encode applies the batch and re-serializes to the
/// original format. All sample work delegates to the [`AudioBuffer`] engine.
#[derive(Debug, Component)]
pub(super) struct AudioStream {
    pub(super) clip: AudioBuffer,
    #[component(name)]
    pub(super) format_id: FormatId,
    /// The clip's duration, rounded down to the millisecond. Redaction keeps
    /// the source's timeline, so it holds for every location.
    duration_ms: u64,
}

impl AudioStream {
    /// The stream over `clip`.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the clip's duration cannot be read.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn new(clip: AudioBuffer, format_id: FormatId) -> Result<Self> {
        let duration_ms = clip.duration_ms()?;
        Ok(Self {
            clip,
            format_id,
            duration_ms,
        })
    }

    /// Check that `location` starts within the clip.
    ///
    /// The duration is rounded down, so a location starting in the clip's
    /// last, partial millisecond is within it.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the location starts past the clip's end: it
    /// addresses no audio, so redacting it would leave the content as it was.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn within(&self, location: &AudioLocation) -> Result<()> {
        if location.span.start_millis() <= self.duration_ms {
            return Ok(());
        }
        Err(Error::malformed(format!(
            "span {}..{} ms is outside the clip (0..{} ms)",
            location.span.start_millis(),
            location.span.end_millis(),
            self.duration_ms
        )))
    }
}

impl Stream<Audio> for AudioStream {
    fn chunks(&self) -> Result<Vec<Chunk<Audio>>> {
        Ok(vec![Chunk {
            location: AudioLocation::from_millis(0, self.duration_ms),
            data: AudioData::new(self.clip.encode()?, self.clip.format()),
            hints: Vec::new(),
        }])
    }
}

#[async_trait::async_trait]
impl DataReader<Audio> for AudioStream {
    async fn read_at(&self, location: &AudioLocation) -> Result<AudioData> {
        self.within(location)?;
        // The whole clip is the addressable unit; a partial time range still
        // resolves to the full audio for downstream extraction.
        Ok(AudioData::new(self.clip.encode()?, self.clip.format()))
    }
}

#[async_trait::async_trait]
impl DataWriter<Audio> for AudioStream {
    async fn write_at(&mut self, redactions: Redactions<Audio>) -> Result<()> {
        for (location, _) in redactions.iter() {
            self.within(location)?;
        }
        self.clip.redact_batch(redactions);
        Ok(())
    }
}

#[cfg(all(test, feature = "wav"))]
mod tests {
    use elide_core::ErrorKind;

    use super::*;
    use crate::fixtures;
    use crate::modality::{AudioFormat, AudioReplacement};

    fn stream() -> AudioStream {
        let clip = AudioBuffer::open(&fixtures::wav_ramp(1), AudioFormat::Wav).unwrap();
        AudioStream::new(clip, FormatId::new("elide.audio.wav")).unwrap()
    }

    #[tokio::test]
    async fn a_location_outside_the_clip_is_an_error() {
        let mut stream = stream();
        let inside = AudioLocation::from_millis(100, 200);
        for location in [&inside, &AudioLocation::from_millis(0, 0)] {
            assert!(stream.read_at(location).await.is_ok(), "{location:?}");
        }
        for outside in [
            AudioLocation::from_millis(1001, 1500),
            AudioLocation::from_millis(1500, 1500),
        ] {
            let err = stream.read_at(&outside).await.unwrap_err();
            assert_eq!(err.kind(), ErrorKind::MalformedInput, "{outside:?}");
            let batch = Redactions::from([
                (inside.clone(), AudioReplacement::Silenced),
                (outside.clone(), AudioReplacement::Silenced),
            ]);
            let err = stream.write_at(batch).await.unwrap_err();
            assert_eq!(err.kind(), ErrorKind::MalformedInput, "{outside:?}");
        }
        assert_eq!(stream.clip.encode().unwrap(), fixtures::wav_ramp(1));
    }
}
