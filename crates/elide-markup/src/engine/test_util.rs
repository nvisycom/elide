//! What the engine's tests share: a body stream over a document read by a
//! config, and what a test reads out of it.

use elide_codec::extract::ExtractStream;
use elide_codec::{FormatId, Stream};
use elide_core::modality::Chunk;
use elide_core::modality::text::Text;

use super::address::{MarkupAddresser, MarkupSpan};
use super::config::MarkupConfig;
use super::parser::build_items;

/// The [`FormatId`] a test stream is under.
pub(crate) const TEST_FORMAT: FormatId = FormatId::new("elide.test.markup");

/// A small synthetic block vocabulary. The real HTML vocabulary is exercised
/// by the HTML loader's own tests; here it is a stand-in so a test targets the
/// engine, not the HTML element lists.
#[cfg(feature = "html")]
pub(crate) const TEST_BLOCKS: &[&str] = &["p", "div"];

/// The body stream of `raw` read by `config`.
pub(crate) fn stream(raw: &str, config: MarkupConfig<'_>) -> ExtractStream<MarkupSpan> {
    let items = build_items(raw, config).expect("markup decode succeeds");
    ExtractStream::new(TEST_FORMAT, items, MarkupAddresser)
}

/// The text of each chunk `stream` yields.
pub(crate) fn values(stream: &ExtractStream<MarkupSpan>) -> Vec<String> {
    stream
        .chunks()
        .unwrap()
        .into_iter()
        .map(|chunk| chunk.data.as_str().to_owned())
        .collect()
}

/// The text of each hint of `chunk`.
pub(crate) fn hint_texts(chunk: &Chunk<Text>) -> Vec<String> {
    chunk
        .hints
        .iter()
        .map(|hint| hint.data.as_str().to_owned())
        .collect()
}
