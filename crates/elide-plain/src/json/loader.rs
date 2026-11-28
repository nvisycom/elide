//! JSON loader: decode the source bytes, strip the terminal control sequences
//! its strings carry, and lex the rest into a [`JsonHandler`], which keeps the
//! formatting (indentation, key order, whitespace) verbatim.

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::JsonHandler;
use super::handler::FORMAT_ID;
use crate::control::Stripped;
use crate::loaded::Loaded;

/// Loader for JSON files. Produces one leaf [`Document`] per input.
#[derive(Debug)]
pub(crate) struct JsonLoader;

impl JsonLoader {
    /// The handler over `content`, each key and string value without the
    /// terminal control sequences it decodes to ([`control`](crate::control)).
    pub(crate) fn load(&self, content: ContentData) -> Result<Loaded<JsonHandler>> {
        let Stripped { kept, removed } =
            JsonHandler::parse(&content.decode()?)?.strip_controls()?;
        Ok(Loaded::new(kept, removed))
    }
}

#[async_trait::async_trait]
impl DocumentLoader for JsonLoader {
    /// Terminal control sequences are always stripped: they are undisplayed
    /// content, which no policy chooses for.
    async fn decode(&self, content: ContentData, _policy: HiddenContentPolicy) -> Result<Document> {
        Ok(self.load(content)?.into_document(FORMAT_ID))
    }
}

#[cfg(test)]
mod tests {
    use elide_codec::Encode;
    use elide_core::redaction::HiddenContent;
    use elide_core::{Component, ErrorKind};

    use super::*;

    #[test]
    fn load_valid_json() -> Result<()> {
        let Loaded {
            handler: doc,
            stripped,
        } = JsonLoader.load(ContentData::from_text(r#"{"a":1}"#))?;
        assert_eq!(doc.id().name, "elide.text.json");
        assert!(stripped.is_empty());
        Ok(())
    }

    #[test]
    fn load_rejects_malformed_json() {
        let err = JsonLoader
            .load(ContentData::from_text(r#"{"a":}"#))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    /// A control sequence a string decodes to goes, written escaped or raw,
    /// in a value or a key; the rest of the source stays as written.
    #[test]
    fn terminal_control_sequences_are_stripped() -> Result<()> {
        let src =
            "{\n  \"na\\u001b[mme\": \"Ja\\u001b[mne \\u00e9\",\n  \"n\": 1, \"t\": \"x\u{7f}\"\n}";
        let Loaded {
            handler: doc,
            stripped,
        } = JsonLoader.load(ContentData::from_text(src))?;
        assert_eq!(
            doc.encode()?.decode()?,
            "{\n  \"name\": \"Jane \u{e9}\",\n  \"n\": 1, \"t\": \"x\"\n}"
        );
        let stripped: Vec<_> = stripped
            .iter()
            .map(|entry| (entry.content, entry.count))
            .collect();
        assert_eq!(stripped, [(HiddenContent::Undisplayed, 3)]);
        Ok(())
    }
}
