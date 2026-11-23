//! Plain-text loader: validates raw text, strips the terminal control
//! sequences in it, and parses the rest into a [`TxtHandler`].

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::TxtHandler;
use super::handler::FORMAT_ID;
use crate::control::{self, Stripped};
use crate::loaded::Loaded;

/// Loader that validates and parses plain-text files. Produces one leaf
/// [`Document`] per input.
#[derive(Debug)]
pub(crate) struct TxtLoader;

impl TxtLoader {
    /// The handler over `content`'s text without its terminal control
    /// sequences ([`control`]).
    pub(crate) fn load(&self, content: ContentData) -> Result<Loaded<TxtHandler>> {
        let Stripped { kept, removed } = control::strip_owned(content.decode()?);
        Ok(Loaded::new(TxtHandler::new(kept), removed))
    }
}

#[async_trait::async_trait]
impl DocumentLoader for TxtLoader {
    /// Terminal control sequences are always stripped: they are undisplayed
    /// content, which no policy chooses for.
    async fn decode(&self, content: ContentData, _policy: HiddenContentPolicy) -> Result<Document> {
        Ok(self.load(content)?.into_document(FORMAT_ID))
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use elide_codec::Stream;
    use elide_core::ErrorKind;
    use elide_core::redaction::HiddenContent;

    use super::*;

    #[test]
    fn load_multiline() -> Result<()> {
        let Loaded {
            handler: doc,
            stripped,
        } = TxtLoader.load(ContentData::from_text("hello\nworld\n"))?;
        assert_eq!(doc.format().as_str(), "elide.text.txt");
        // The source is held verbatim, blank lines and trailing newline included.
        assert_eq!(doc.text(), "hello\nworld\n");
        assert!(stripped.is_empty());
        Ok(())
    }

    #[test]
    fn load_no_trailing_newline() -> Result<()> {
        let doc = TxtLoader
            .load(ContentData::from_text("single line"))?
            .handler;
        assert_eq!(doc.text(), "single line");
        Ok(())
    }

    #[test]
    fn load_invalid_utf8() {
        let content = ContentData::new(Bytes::from_static(&[0xFF, 0xFE, 0x00]));
        let err = TxtLoader.load(content).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    /// A control sequence inside a name goes, so detection reads the name a
    /// terminal shows, and the strip is recorded.
    #[test]
    fn terminal_control_sequences_are_stripped() -> Result<()> {
        let Loaded {
            handler: doc,
            stripped,
        } = TxtLoader.load(ContentData::from_text(
            "call Ja\u{1b}[mne Roe\u{1b}]0;x\u{7}\n",
        ))?;
        assert_eq!(doc.text(), "call Jane Roe\n");
        let stripped: Vec<_> = stripped
            .iter()
            .map(|entry| (entry.content, entry.count))
            .collect();
        assert_eq!(stripped, [(HiddenContent::Undisplayed, 2)]);
        Ok(())
    }
}
