//! [`AudioStream`]: the clip body part — streams the whole clip as one chunk,
//! accumulates redactions, and re-encodes on demand.

use elide_codec::content::ContentData;
use elide_codec::{FormatId, Stream};
use elide_core::Result;
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use crate::AudioBuffer;
use crate::modality::{Audio, AudioData, AudioLocation};

/// The clip stream part. Holds the clip as an [`AudioBuffer`]; redaction
/// accumulates on it and `encode` applies the batch and re-serializes to the
/// original format. All sample work delegates to the [`AudioBuffer`] engine.
#[derive(Debug)]
pub(super) struct AudioStream {
    pub(super) clip: AudioBuffer,
    pub(super) format_id: FormatId,
}

impl Stream<Audio> for AudioStream {
    fn format(&self) -> FormatId {
        self.format_id
    }

    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::new(self.clip.encode()?))
    }

    fn chunks(&self) -> Result<Vec<Chunk<Audio>>> {
        let total_ms = self.clip.duration_ms()?;
        Ok(vec![Chunk {
            location: AudioLocation::from_millis(0, total_ms),
            data: AudioData::new(self.clip.encode()?, self.clip.format()),
            hints: Vec::new(),
        }])
    }
}

#[async_trait::async_trait]
impl DataReader<Audio> for AudioStream {
    async fn read_at(&self, _location: &AudioLocation) -> Result<Option<AudioData>> {
        // The whole clip is the addressable unit; a partial time range still
        // resolves to the full audio for downstream extraction.
        Ok(Some(AudioData::new(
            self.clip.encode()?,
            self.clip.format(),
        )))
    }
}

#[async_trait::async_trait]
impl DataWriter<Audio> for AudioStream {
    async fn write_at(&mut self, redactions: Redactions<Audio>) -> Result<()> {
        self.clip.redact_batch(redactions);
        Ok(())
    }
}
