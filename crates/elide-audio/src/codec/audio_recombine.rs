//! [`AudioRecombine`]: the clip's one part is the clip, checked for tags.

use elide_codec::content::ContentData;
use elide_codec::{EncodedPart, LeafRecombine, LocalId, Recombine};
use elide_core::{Error, ErrorKind, Result};

use crate::AudioBuffer;
use crate::modality::AudioFormat;

/// The recombiner for an audio clip: its one stream part's bytes are the clip,
/// and the encoded clip must carry no tag block.
pub(super) struct AudioRecombine {
    /// The clip's container format, to read the encoded output's tags.
    pub(super) format: AudioFormat,
}

impl Recombine for AudioRecombine {
    fn assemble(&self, parts: &[EncodedPart], removed: &[LocalId]) -> Result<ContentData> {
        LeafRecombine.assemble(parts, removed)
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        if AudioBuffer::carries_metadata(output.as_bytes(), self.format)? {
            return Err(Error::new(
                ErrorKind::Sanitization,
                "encoded audio still carries a tag block",
            ));
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "wav"))]
mod tests {
    use elide_codec::Recombine;
    use elide_codec::content::ContentData;
    use elide_core::ErrorKind;

    use super::AudioRecombine;
    use crate::fixtures;
    use crate::modality::AudioFormat;

    #[test]
    fn a_tagged_output_is_refused_as_a_sanitization_failure() {
        let mut tagged = fixtures::wav_ramp(1).to_vec();
        tagged.extend_from_slice(b"LIST\x12\x00\x00\x00INFOIART\x06\x00\x00\x00Jane R");
        let riff_size = (tagged.len() - 8) as u32;
        tagged[4..8].copy_from_slice(&riff_size.to_le_bytes());

        let recombine = AudioRecombine {
            format: AudioFormat::Wav,
        };
        let error = recombine
            .validate(&ContentData::from(tagged))
            .expect_err("tagged output");
        assert_eq!(error.kind(), ErrorKind::Sanitization);
    }
}
