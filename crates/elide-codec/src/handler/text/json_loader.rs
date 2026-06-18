//! JSON loader: decode source bytes and hand them to [`JsonHandler`]
//! verbatim. Formatting (indentation, key order, trailing whitespace) is
//! preserved by the handler's slot model; the loader only does encoding +
//! well-formedness checks.

use elide_core::modality::text::Text;
use elide_core::{Error, ErrorKind};

use super::JsonHandler;
use crate::Loader;
use crate::content::ContentData;

/// Loader for JSON files. Produces one [`JsonHandler`] per input.
#[derive(Debug)]
pub struct JsonLoader;

impl Loader<Text> for JsonLoader {
    type Handler = JsonHandler;

    async fn decode(&self, content: ContentData) -> Result<JsonHandler, Error> {
        let text = content.decode()?;
        // Validate well-formedness eagerly; the handler's lexer re-parses
        // but with a friendlier error path. Reject here so callers get a
        // single decode-time validation point.
        serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| Error::new(ErrorKind::Validation, format!("invalid JSON: {e}")))?;
        Ok(JsonHandler::from_source_string(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Handler;

    #[tokio::test]
    async fn load_valid_json() -> Result<(), Error> {
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
        assert!(err.to_string().contains("JSON"));
    }
}
