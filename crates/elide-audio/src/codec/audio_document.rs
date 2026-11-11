//! [`AudioDocument`]: a decoded clip, encoded as itself and checked for tags.

use elide_codec::content::ContentData;
use elide_codec::{Blob, LocalId, Native, StreamMut};
use elide_core::{Error, Result};

use super::audio_loader::BODY_PART_ID;
use super::audio_stream::AudioStream;
use crate::AudioBuffer;
use crate::modality::{Audio, AudioFormat};

/// A decoded clip: its one stream is the clip, and the encoded clip must carry
/// no tag block.
pub(super) struct AudioDocument {
    /// The clip.
    pub(super) clip: AudioStream,
    /// The clip's container format, to read the encoded output's tags.
    pub(super) format: AudioFormat,
}

impl Native for AudioDocument {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        vec![(
            LocalId::new(BODY_PART_ID),
            StreamMut::new::<Audio, _>(&mut self.clip),
        )]
    }

    fn encode(&self, _blobs: &[Blob], _removed: &[LocalId]) -> Result<ContentData> {
        Ok(ContentData::new(self.clip.clip.encode()?))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        if AudioBuffer::carries_metadata(output.as_bytes(), self.format)? {
            return Err(Error::sanitization(
                "encoded audio still carries a tag block",
            ));
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "wav"))]
mod tests {
    use elide_codec::content::ContentData;
    use elide_codec::{FormatId, Native};
    use elide_core::ErrorKind;

    use super::AudioDocument;
    use crate::codec::audio_stream::AudioStream;
    use crate::modality::AudioFormat;
    use crate::{AudioBuffer, fixtures};

    #[test]
    fn a_tagged_output_is_refused_as_a_sanitization_failure() {
        let mut tagged = fixtures::wav_ramp(1).to_vec();
        tagged.extend_from_slice(b"LIST\x12\x00\x00\x00INFOIART\x06\x00\x00\x00Jane R");
        let riff_size = (tagged.len() - 8) as u32;
        tagged[4..8].copy_from_slice(&riff_size.to_le_bytes());

        let clip = AudioBuffer::open(&fixtures::wav_ramp(1), AudioFormat::Wav).unwrap();
        let document = AudioDocument {
            clip: AudioStream {
                clip,
                format_id: FormatId::new("elide.audio.wav"),
            },
            format: AudioFormat::Wav,
        };
        let error = document
            .validate(&ContentData::from(tagged))
            .expect_err("tagged output");
        assert_eq!(error.kind(), ErrorKind::Sanitization);
    }
}
