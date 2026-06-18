//! Plain-text loader: validates and parses raw text content into a
//! [`TxtHandler`].

use elide_core::Error;
use elide_core::modality::text::Text;

use super::TxtHandler;
use crate::Loader;
use crate::content::ContentData;

/// Loader that validates and parses plain-text files. Produces one
/// [`TxtHandler`] per input.
#[derive(Debug)]
pub(crate) struct TxtLoader;

impl Loader<Text> for TxtLoader {
    type Handler = TxtHandler;

    async fn decode(&self, content: ContentData) -> Result<TxtHandler, Error> {
        let text = content.decode()?;
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
        let doc = TxtLoader
            .decode(ContentData::from_text("hello\nworld\n"))
            .await?;
        assert_eq!(doc.format().as_str(), "elide.text.txt");
        assert_eq!(doc.lines(), &["hello", "world"]);
        assert!(doc.trailing_newline());
        Ok(())
    }

    #[tokio::test]
    async fn load_no_trailing_newline() -> Result<(), Error> {
        let doc = TxtLoader
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
        let err = TxtLoader.decode(content).await.unwrap_err();
        assert!(err.to_string().contains("UTF-8"));
    }
}
