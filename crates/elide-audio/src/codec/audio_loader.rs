//! [`AudioDocumentLoader`]: decode a clip into a one-stream [`Document`], its
//! tags stripped under the hidden-content policy.

use elide_codec::content::ContentData;
use elide_codec::{
    Document, DocumentLoader, DocumentPart, ErasedStream, FormatId, LocalId, Stream,
    StrippedContent,
};
use elide_core::Result;
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};

use super::audio_recombine::AudioRecombine;
use super::audio_stream::AudioStream;
use crate::AudioBuffer;
use crate::modality::{Audio, AudioFormat};

/// The id of a clip's one stream part.
const BODY_PART_ID: &str = "body";

/// A loader that decodes a clip into a [`Document`] of one [`AudioStream`].
///
/// An audio clip's tags (ID3, APE, `LIST/INFO`) are not surfaced to detection,
/// so they are stripped at decode whatever the [`HiddenContentPolicy`] asks: a
/// format that cannot redact hidden content strips it.
pub(crate) struct AudioDocumentLoader {
    format_id: FormatId,
    format: AudioFormat,
}

impl AudioDocumentLoader {
    /// A loader for `format_id` / `format`.
    pub(crate) fn new(format_id: FormatId, format: AudioFormat) -> Self {
        Self { format_id, format }
    }
}

#[async_trait::async_trait]
impl DocumentLoader for AudioDocumentLoader {
    async fn decode(&self, content: ContentData, _policy: HiddenContentPolicy) -> Result<Document> {
        // The registry routed to this loader for its one format; open the bytes
        // as exactly that, no re-detection.
        let mut clip = AudioBuffer::open(content.as_bytes(), self.format)?;
        let mut stripped = StrippedContent::new();
        stripped.record(HiddenContent::Properties, None, clip.strip_metadata()?);
        let stream: Box<dyn Stream<Audio>> = Box::new(AudioStream {
            clip,
            format_id: self.format_id.clone(),
        });
        let part = DocumentPart::Stream {
            id: LocalId::new(BODY_PART_ID),
            handle: ErasedStream::new(self.format_id.clone(), stream),
        };
        let recombine = AudioRecombine {
            format: self.format,
        };
        Ok(
            Document::new(self.format_id.clone(), vec![part], Box::new(recombine))
                .with_stripped(stripped),
        )
    }
}
