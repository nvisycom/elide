//! XML handler side: the [`XmlHandler`] type, its [`Format`] descriptor, and the
//! [`XmlEncoder`] that re-serializes a mutated [`ExtractedItem`] stream — the
//! shared markup engine HTML runs on too.
//!
//! The encoder preserves the document **verbatim**: it splices each item's
//! current value back at its recorded source byte span into the retained raw
//! string, leaving the declaration, whitespace, attribute quoting, and
//! everything outside the redacted spans byte-identical. Splices apply
//! right-to-left so an earlier edit's length delta never shifts a later span.
//!
//! [`ExtractedItem`]: super::ExtractedItem

use std::cmp::Reverse;
use std::ops::Range;

use elide_core::modality::text::{SourceRef, Text};
use elide_core::{Error, ErrorKind, Result};

use super::XmlLoader;
use crate::content::ContentData;
use crate::handler::extract::{Encoder, ExtractHandler, ExtractedItem};
use crate::{Format, FormatId};

/// Stable [`FormatId`] for the XML codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.xml");

/// Handler type for loaded XML (and HTML) content.
pub(crate) type XmlHandler = ExtractHandler<XmlEncoder>;

/// An XML [`ExtractedItem`] addressed by the source byte span its
/// `value` occupies in the original document.
///
/// [`ExtractedItem`]: super::ExtractedItem
pub(crate) type XmlItem = ExtractedItem<XmlSpan>;

/// The source byte span (in the retained raw document) that a
/// [`ExtractedItem`]'s value occupies: the region the encoder
/// overwrites. These are the *inner* bytes: a text node's text, an
/// attribute value between the quotes, a comment body between `<!--` and
/// `-->`, a CDATA payload between `<![CDATA[` and `]]>`.
///
/// [`ExtractedItem`]: super::ExtractedItem
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XmlSpan(pub(super) Range<usize>);

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), XmlLoader)
        .with_extensions(["xml"])
        .with_content_types(["application/xml", "text/xml"])
}

/// Re-serializes a mutated item stream by splicing each value back at its
/// source span into the retained raw document.
#[derive(Debug)]
pub(crate) struct XmlEncoder {
    pub(super) raw: String,
}

impl Encoder for XmlEncoder {
    type Address = XmlSpan;

    fn encode(&self, items: &[XmlItem]) -> Result<ContentData> {
        let out = splice(&self.raw, items)?;
        Ok(ContentData::new(out.into_bytes().into()))
    }

    fn source_span(&self, item: &XmlItem, local: Range<usize>) -> Vec<SourceRef> {
        // The item's value is the verbatim source slice at its `XmlSpan`, so a
        // byte offset into the value is the same offset into the source span —
        // the mapping is a simple add, bounded by the span's end. Single file:
        // no part. At most one range: the value carries no entity to split on.
        let base = &item.address.0;
        let mapped = base
            .start
            .checked_add(local.start)
            .zip(base.start.checked_add(local.end))
            .filter(|&(start, end)| start <= end && end <= base.end)
            .map(|(start, end)| SourceRef::new(start..end));
        mapped.into_iter().collect()
    }

    fn locate_source(
        &self,
        items: &[ExtractedItem<XmlSpan>],
        source: &[SourceRef],
    ) -> Option<(usize, Range<usize>)> {
        // Inverse of `source_span`: the item value is the verbatim source slice,
        // so a raw offset within the item's span is the same offset in the value
        // (a subtract). Single file, so the reference carries no part. Cover the
        // raw span from the first ref's start to the last ref's end.
        let raw_start = source.iter().map(|s| s.range.start).min()?;
        let raw_end = source.iter().map(|s| s.range.end).max()?;
        let (i, base) = items
            .iter()
            .map(|item| &item.address.0)
            .enumerate()
            .find(|(_, base)| base.start <= raw_start && raw_end <= base.end)?;
        Some((i, (raw_start - base.start)..(raw_end - base.start)))
    }
}

/// Splice each item's current value back at its source span into `raw`,
/// returning the rebuilt string. Shared by the XML encoder and by
/// container formats (DOCX) that redact an XML part and re-pack it.
///
/// Item spans come from disjoint quick-xml events over this same `raw`, so
/// they never overlap. Applying them right-to-left means each splice's
/// length delta can't shift the spans of items earlier in the document.
pub(crate) fn splice(raw: &str, items: &[XmlItem]) -> Result<String> {
    let mut ordered: Vec<&XmlItem> = items.iter().collect();
    ordered.sort_by_key(|item| Reverse(item.address.0.start));

    let mut out = raw.to_owned();
    for item in ordered {
        let Range { start, end } = item.address.0.clone();
        // Spans index into `out`, which starts as `raw` and only ever
        // grows/shrinks to the right of the current splice, so they stay
        // in-bounds and on char boundaries by construction. The guards are
        // defensive: a malformed loader would surface here rather than
        // panic in `replace_range`.
        if end > out.len() || start > end {
            return Err(Error::new(
                ErrorKind::Processing,
                format!(
                    "xml splice span {start}..{end} out of bounds (len {})",
                    out.len()
                ),
            ));
        }
        if !out.is_char_boundary(start) || !out.is_char_boundary(end) {
            return Err(Error::new(
                ErrorKind::Processing,
                format!("xml splice span {start}..{end} falls mid-character"),
            ));
        }
        // `value` is the raw on-the-wire slice (the loader stores source
        // bytes verbatim, never a decoded form), so it splices back with
        // no escape transform: only the redacted sub-range ever changed.
        out.replace_range(start..end, &item.value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use elide_core::modality::DataWriter;
    use elide_core::modality::text::{TextLocation, TextReplacement};
    use elide_core::operator::Redactions;

    use super::*;
    #[cfg(feature = "html")]
    use crate::handler::markup::config::MarkupConfig;
    #[cfg(feature = "html")]
    use crate::handler::markup::markup_parser::build_items;
    use crate::{Handler, Loader};

    async fn load(raw: &str) -> XmlHandler {
        XmlLoader
            .decode(ContentData::from_text(raw))
            .await
            .expect("xml decode succeeds")
    }

    /// Build a handler over `raw` with an explicit (lenient HTML) config, for
    /// the round-trips that need the skip/void tolerance the strict loader
    /// won't grant.
    #[cfg(feature = "html")]
    fn handler_with(raw: &str, config: MarkupConfig<'_>) -> XmlHandler {
        let items = build_items(raw, config).expect("markup decode succeeds");
        ExtractHandler::new(
            FORMAT_ID.clone(),
            XmlEncoder {
                raw: raw.to_owned(),
            },
            items,
        )
    }

    fn encoded(h: &XmlHandler) -> String {
        h.encode().unwrap().decode().unwrap()
    }

    #[tokio::test]
    async fn encode_unchanged_round_trips_verbatim() {
        let raw = "<?xml version=\"1.0\"?>\n<root attr=\"x\">\n  <name>Alice</name>\n  <!-- note -->\n</root>\n";
        let h = load(raw).await;
        assert_eq!(encoded(&h), raw);
    }

    #[tokio::test]
    async fn round_trips_verbatim_across_tricky_inputs() {
        for raw in [
            "\u{FEFF}<?xml version=\"1.0\"?><r>x</r>",
            "  <r>x</r>",
            "<r>café résumé</r>",
            "<r>a&amp;b</r>",
            "<r><![CDATA[üñ]]></r>",
        ] {
            let h = load(raw).await;
            assert_eq!(encoded(&h), raw, "round-trip changed: {raw:?}");
        }
    }

    #[tokio::test]
    async fn lift_carries_the_exact_source_span() {
        // Text preceded by tags: the chunk's stream offset differs from the raw
        // byte offset, so `source` must point at the raw bytes, not the stream.
        let raw = "<root><name>Alice Carter</name></root>";
        let mut h = load(raw).await;
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str() == "Alice Carter" {
                break c;
            }
        };
        // Redact "Carter" — value-local [6, 12).
        let lifted = h.lift(&chunk, TextLocation::new(6, 12)).expect("in bounds");
        // "Carter" sits at raw bytes 18..24 in the document.
        let want = "<root><name>Alice Carter".find("Carter").unwrap();
        assert_eq!(
            lifted.source,
            vec![SourceRef::new(want..want + "Carter".len())]
        );
        assert_eq!(&raw[want..want + 6], "Carter");
    }

    #[tokio::test]
    async fn redact_text_node() {
        let raw = "<root><name>Alice</name></root>";
        let mut h = load(raw).await;
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str() == "Alice" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[NAME]"));
        h.write_at(rs).await.unwrap();
        assert_eq!(encoded(&h), "<root><name>[NAME]</name></root>");
    }

    #[tokio::test]
    async fn redact_attribute_value() {
        let raw = r#"<user email="alice@example.com">Bob</user>"#;
        let mut h = load(raw).await;
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str() == "alice@example.com" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[EMAIL]"));
        h.write_at(rs).await.unwrap();
        assert_eq!(encoded(&h), r#"<user email="[EMAIL]">Bob</user>"#);
    }

    #[tokio::test]
    async fn redact_cdata_body() {
        let raw = "<doc><![CDATA[alice@example.com]]></doc>";
        let mut h = load(raw).await;
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str() == "alice@example.com" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[EMAIL]"));
        h.write_at(rs).await.unwrap();
        assert_eq!(encoded(&h), "<doc><![CDATA[[EMAIL]]]></doc>");
    }

    #[tokio::test]
    async fn redact_partial_text() {
        let raw = "<p>contact alice@example.com today</p>";
        let mut h = load(raw).await;
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str().contains("alice@example.com") {
                break c;
            }
        };
        let at = chunk.data.as_str().find("alice@example.com").unwrap();
        let loc = TextLocation::new(
            chunk.location.range.start + at,
            chunk.location.range.start + at + "alice@example.com".len(),
        );
        let mut rs = Redactions::new();
        rs.push(loc, TextReplacement::substituted("[EMAIL]"));
        h.write_at(rs).await.unwrap();
        assert_eq!(encoded(&h), "<p>contact [EMAIL] today</p>");
    }

    // A lenient round-trip over the same encoder, exercising our splice
    // bookkeeping across a skipped region. Gated on `html`: the lenient config
    // only exists when HTML is compiled. A small synthetic vocabulary stands in
    // for the real HTML element lists, which the HTML loader tests directly.
    #[cfg(feature = "html")]
    const TEST_BLOCKS: &[&str] = &["p", "div"];

    #[cfg(feature = "html")]
    #[tokio::test]
    async fn redaction_after_a_skipped_script_is_byte_faithful() {
        // Redacting text that follows a skipped script leaves the script and all
        // other markup byte-identical, changing only the targeted span — the
        // splice offsets stay correct across the skipped region.
        let raw = r#"<p><script>var a="keep@x.com";</script>mail alice@example.com</p>"#;
        let mut h = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &["script"]));
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str().contains("alice@example.com") {
                break c;
            }
        };
        let at = chunk.data.as_str().find("alice@example.com").unwrap();
        let loc = TextLocation::new(
            chunk.location.range.start + at,
            chunk.location.range.start + at + "alice@example.com".len(),
        );
        let mut rs = Redactions::new();
        rs.push(loc, TextReplacement::substituted("[EMAIL]"));
        h.write_at(rs).await.unwrap();
        assert_eq!(
            encoded(&h),
            r#"<p><script>var a="keep@x.com";</script>mail [EMAIL]</p>"#
        );
    }
}
