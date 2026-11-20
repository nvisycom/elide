//! XML loader: rebuilds decoded content as parsed (without processing
//! instructions, its document type, XInclude, or, under the policy, comments) and
//! reads the rest with the shared markup engine ([`decode`]) into a leaf
//! [`Document`] whose body is an [`ExtractStream`] re-serialised by
//! [`MarkupDocument`].
//!
//! [`decode`]: crate::engine::document::decode
//! [`ExtractStream`]: elide_codec::extract::ExtractStream
//! [`MarkupDocument`]: crate::engine::document::MarkupDocument

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::handler::FORMAT_ID;
use crate::engine::config::MarkupConfig;
use crate::engine::document::decode;

/// Loader for XML files. Produces one leaf [`Document`] per input.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct XmlLoader;

#[async_trait::async_trait]
impl DocumentLoader for XmlLoader {
    async fn decode(&self, content: ContentData, policy: HiddenContentPolicy) -> Result<Document> {
        decode(FORMAT_ID, MarkupConfig::xml(), content, policy)
    }
}
