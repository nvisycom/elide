//! JSON loader: decode the source bytes and lex them into a [`JsonHandler`],
//! which keeps the formatting (indentation, key order, whitespace) verbatim.

use elide_codec::Loader;
use elide_codec::content::ContentData;
use elide_core::Result;
use elide_core::modality::text::Text;

use super::JsonHandler;

/// Loader for JSON files. Produces one [`JsonHandler`] per input.
#[derive(Debug)]
pub(crate) struct JsonLoader;

#[async_trait::async_trait]
impl Loader for JsonLoader {
    type Modality = Text;
    type Stream = JsonHandler;

    async fn decode(&self, content: ContentData) -> Result<JsonHandler> {
        JsonHandler::parse(&content.decode()?)
    }
}

#[cfg(test)]
mod tests {
    use elide_codec::Stream;
    use elide_core::ErrorKind;

    use super::*;

    #[tokio::test]
    async fn load_valid_json() -> Result<()> {
        let doc = JsonLoader
            .decode(ContentData::from_text(r#"{"a":1}"#))
            .await?;
        assert_eq!(doc.format().as_str(), "elide.text.json");
        Ok(())
    }

    #[tokio::test]
    async fn load_rejects_malformed_json() {
        let err = JsonLoader
            .decode(ContentData::from_text(r#"{"a":}"#))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }
}
