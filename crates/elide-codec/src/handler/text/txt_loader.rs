//! Plain-text loader: validates and parses raw text content into a
//! [`TxtHandler`].

use elide_core::Result;
use elide_core::modality::text::Text;

use super::TxtHandler;
use crate::Loader;
use crate::content::ContentData;

/// Loader that validates and parses plain-text files. Produces one
/// [`TxtHandler`] per input.
#[derive(Debug)]
pub(crate) struct TxtLoader;

#[async_trait::async_trait]
impl Loader<Text> for TxtLoader {
    type Handler = TxtHandler;

    async fn decode(&self, content: ContentData) -> Result<TxtHandler> {
        Ok(TxtHandler::new(content.decode()?))
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::Handler;

    #[tokio::test]
    async fn load_multiline() -> Result<()> {
        let doc = TxtLoader
            .decode(ContentData::from_text("hello\nworld\n"))
            .await?;
        assert_eq!(doc.format().as_str(), "elide.text.txt");
        // The source is held verbatim, blank lines and trailing newline included.
        assert_eq!(doc.text(), "hello\nworld\n");
        Ok(())
    }

    #[tokio::test]
    async fn load_no_trailing_newline() -> Result<()> {
        let doc = TxtLoader
            .decode(ContentData::from_text("single line"))
            .await?;
        assert_eq!(doc.text(), "single line");
        Ok(())
    }

    #[tokio::test]
    async fn load_invalid_utf8() {
        let content = ContentData::new(Bytes::from_static(&[0xFF, 0xFE, 0x00]));
        let err = TxtLoader.decode(content).await.unwrap_err();
        assert!(err.to_string().contains("UTF-8"));
    }
}
