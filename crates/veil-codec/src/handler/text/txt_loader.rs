//! Plain-text loader: validates and parses raw text content into a
//! [`TxtHandler`].

use veil_core::Error;
use veil_core::modality::text::Text;

use super::TxtHandler;
use crate::Loader;
use crate::content::{ContentData, TextEncoding};

/// Loader that validates and parses plain-text files. Produces one
/// [`TxtHandler`] per input.
#[derive(Debug, Default)]
pub struct TxtLoader {
    /// Character encoding of the input bytes. Defaults to UTF-8.
    pub encoding: TextEncoding,
}

impl Loader<Text> for TxtLoader {
    type Handler = TxtHandler;

    async fn decode(&self, content: ContentData) -> Result<TxtHandler, Error> {
        let raw = content.to_bytes();
        let text = self.encoding.decode_bytes(&raw)?;
        let trailing_newline = text.ends_with('\n');
        let lines: Vec<String> = text.lines().map(String::from).collect();
        Ok(TxtHandler::new(lines, trailing_newline))
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::Handler;

    #[tokio::test]
    async fn load_multiline() -> Result<(), Error> {
        let doc = TxtLoader::default()
            .decode(ContentData::from_text("hello\nworld\n"))
            .await?;
        assert_eq!(doc.format().as_str(), "veil.text.txt");
        assert_eq!(doc.lines(), &["hello", "world"]);
        assert!(doc.trailing_newline());
        Ok(())
    }

    #[tokio::test]
    async fn load_no_trailing_newline() -> Result<(), Error> {
        let doc = TxtLoader::default()
            .decode(ContentData::from_text("single line"))
            .await?;
        assert_eq!(doc.len(), 1);
        assert_eq!(doc.line(0), Some("single line"));
        assert!(!doc.trailing_newline());
        Ok(())
    }

    #[tokio::test]
    async fn load_invalid_utf8() {
        let content = ContentData::new(Bytes::from_static(&[0xFF, 0xFE, 0x00]));
        let err = TxtLoader::default().decode(content).await.unwrap_err();
        assert!(err.to_string().contains("UTF-8"));
    }
}
