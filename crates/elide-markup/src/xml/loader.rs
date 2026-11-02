//! XML loader: sanitizes decoded content (its comments, under the policy) and
//! wires the rest through the shared [`parser`] into a leaf [`Document`] whose
//! body is an [`ExtractStream`] recombined by [`MarkupRecombine`].
//!
//! [`parser`]: crate::engine::parser
//! [`ExtractStream`]: elide_codec::extract::ExtractStream
//! [`MarkupRecombine`]: crate::engine::recombine::MarkupRecombine

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::handler::FORMAT_ID;
use crate::engine::config::MarkupConfig;
use crate::engine::parser::build_items;
use crate::engine::recombine::markup_document;
use crate::sanitize::{Dialect, MarkupSanitizer};

/// Loader for XML files. Produces one leaf [`Document`] per input.
#[derive(Debug)]
pub(crate) struct XmlLoader;

#[async_trait::async_trait]
impl DocumentLoader for XmlLoader {
    async fn decode(&self, content: ContentData, policy: HiddenContentPolicy) -> Result<Document> {
        let sanitizer = MarkupSanitizer::new(Dialect::Xml, policy);
        let (text, stripped) = sanitizer.sanitize(content.decode()?)?;
        let items = build_items(&text, MarkupConfig::xml())?;
        Ok(markup_document(FORMAT_ID.clone(), text, items, sanitizer).with_stripped(stripped))
    }
}
