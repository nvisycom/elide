//! [`AudioDocumentLoader`]: decode a clip into a one-stream [`Document`], its
//! tags stripped under the hidden-content policy.

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader, FormatId, StrippedContent};
use elide_core::Result;
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};

use super::audio_document::AudioDocument;
use super::audio_stream::AudioStream;
use crate::AudioBuffer;
use crate::modality::AudioFormat;

/// The id of a clip's one stream.
pub(super) const BODY_PART_ID: &str = "body";

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
        let removed = clip.strip_metadata()?;
        stripped.record(HiddenContent::Properties, None, removed.tags);
        stripped.record(HiddenContent::PriorRevision, None, removed.padding);
        let native = AudioDocument {
            clip: AudioStream {
                clip,
                format_id: self.format_id,
            },
            format: self.format,
        };
        Ok(Document::new(self.format_id, native, Vec::new()).with_stripped(stripped))
    }
}
