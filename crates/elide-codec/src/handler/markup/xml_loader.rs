//! XML loader: parses XML into the shared markup [`ExtractedItem`]
//! stream, recording each item's source byte span so the [`XmlEncoder`]
//! can splice mutated values back **verbatim**.
//!
//! Emits items for element text content, comment bodies, and CDATA
//! payloads, each addressed by an exact source span quick-xml gives us
//! from event positions. Each item's `value` is the raw on-the-wire slice
//! (never a decoded form), so encode is a byte-for-byte splice at the
//! recorded span: the declaration, whitespace, attributes, and everything
//! outside the redacted spans round-trip unchanged.
//!
//! Attribute values are *not* yet redactable: quick-xml's public API
//! exposes an attribute's decoded value but not its source span, and
//! recovering the span by hand means re-implementing the tag parser.
//! Until a clean span is available, attributes pass through untouched.
//!
//! [`ExtractedItem`]: super::ExtractedItem
//! [`XmlEncoder`]: super::XmlEncoder

use std::ops::Range;

use elide_core::modality::text::Text;
use elide_core::{Error, ErrorKind, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

use super::xml_handler::{FORMAT_ID, XmlEncoder, XmlHandler, XmlItem, XmlSpan};
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::{ExtractHandler, ExtractedItem};

/// Loader for XML files. Produces one [`XmlHandler`] per input.
#[derive(Debug)]
pub(crate) struct XmlLoader;

impl Loader<Text> for XmlLoader {
    type Handler = XmlHandler;

    async fn decode(&self, content: ContentData) -> Result<XmlHandler> {
        let text = content.decode()?;
        let items = build_items(&text)?;
        Ok(ExtractHandler::new(
            FORMAT_ID.clone(),
            XmlEncoder { raw: text.clone() },
            items,
        ))
    }
}

/// Extract the redactable XML items from `raw`. Exposed for container
/// formats (DOCX) that run the XML engine over a part they unzip rather
/// than over a whole standalone document.
pub(crate) fn build_items(raw: &str) -> Result<Vec<XmlItem>> {
    let mut reader = Reader::from_str(raw);
    let mut items = Vec::new();
    let mut last = 0usize;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| Error::new(ErrorKind::Validation, format!("malformed XML: {e}")))?;
        // The event occupies raw bytes `[last, pos)`.
        let span = last..reader.buffer_position() as usize;
        last = span.end;

        // The inner (redactable) span of each event, with its delimiters
        // stripped; `None` for events we don't address.
        let inner = match event {
            Event::Eof => break,
            Event::Text(_) => non_blank(raw, span),
            Event::Comment(_) => strip(span, "<!--", "-->"),
            Event::CData(_) => strip(span, "<![CDATA[", "]]>"),
            _ => None,
        };
        if let Some(inner) = inner {
            items.push(span_item(raw, inner));
        }
    }

    Ok(items)
}

/// Return `span` unless it covers only whitespace (those text runs carry
/// no PII and only clutter the stream).
fn non_blank(raw: &str, span: Range<usize>) -> Option<Range<usize>> {
    (!raw[span.clone()].trim().is_empty()).then_some(span)
}

/// Build an item whose value is the verbatim source slice at `span`.
fn span_item(raw: &str, span: Range<usize>) -> XmlItem {
    ExtractedItem {
        value: raw[span.clone()].to_owned(),
        address: XmlSpan(span),
        hints: Vec::new(),
    }
}

/// Narrow `span` by its `open`/`close` delimiters, returning the inner
/// range, or `None` if the span doesn't fit them.
fn strip(span: Range<usize>, open: &str, close: &str) -> Option<Range<usize>> {
    let start = span.start.checked_add(open.len())?;
    let end = span.end.checked_sub(close.len())?;
    (start <= end).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use elide_core::modality::DataWriter;
    use elide_core::modality::text::{TextLocation, TextReplacement};
    use elide_core::operator::Redactions;

    use super::*;
    use crate::Handler;

    async fn load(raw: &str) -> XmlHandler {
        XmlLoader
            .decode(ContentData::from_text(raw))
            .await
            .expect("xml decode succeeds")
    }

    fn encoded(h: &XmlHandler) -> String {
        h.encode().unwrap().decode().unwrap()
    }

    #[tokio::test]
    async fn encode_unchanged_round_trips_verbatim() {
        let raw = "<?xml version=\"1.0\"?>\n<root attr=\"x\">\n  <name>Alice</name>\n  <!-- note -->\n</root>\n";
        let h = load(raw).await;
        // Verbatim: declaration, whitespace, everything preserved.
        assert_eq!(encoded(&h), raw);
    }

    /// The span arithmetic is over raw bytes, so the round-trip must hold
    /// across a BOM, leading whitespace, multibyte UTF-8 text, entity
    /// references, and multibyte CDATA: the cases most likely to break a
    /// byte-offset assumption.
    #[tokio::test]
    async fn round_trips_verbatim_across_tricky_inputs() {
        for raw in [
            "\u{FEFF}<?xml version=\"1.0\"?><r>x</r>", // leading BOM
            "  <r>x</r>",                              // leading whitespace
            "<r>café résumé</r>",                      // multibyte text
            "<r>a&amp;b</r>",                          // entity in text
            "<r><![CDATA[üñ]]></r>",                   // multibyte CDATA
        ] {
            let h = load(raw).await;
            assert_eq!(encoded(&h), raw, "round-trip changed: {raw:?}");
        }
    }

    #[tokio::test]
    async fn stream_yields_text_comment_cdata() {
        let raw =
            r#"<root id="A1"><name>Alice</name><!-- c --><data><![CDATA[secret]]></data></root>"#;
        let mut h = load(raw).await;
        let mut values = Vec::new();
        while let Some(chunk) = h.read_next().await.unwrap() {
            values.push(chunk.data.as_str().to_owned());
        }
        assert!(values.iter().any(|v| v == "Alice"), "text: {values:?}");
        assert!(values.iter().any(|v| v == " c "), "comment: {values:?}");
        assert!(values.iter().any(|v| v == "secret"), "cdata: {values:?}");
        // Attribute values are not yet redactable, so `A1` is not emitted.
        assert!(
            !values.iter().any(|v| v == "A1"),
            "no attr items: {values:?}"
        );
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
        // Redact just the email substring within the text run.
        let text = chunk.data.as_str();
        let at = text.find("alice@example.com").unwrap();
        let loc = TextLocation::new(
            chunk.location.start + at,
            chunk.location.start + at + "alice@example.com".len(),
        );
        let mut rs = Redactions::new();
        rs.push(loc, TextReplacement::substituted("[EMAIL]"));
        h.write_at(rs).await.unwrap();
        assert_eq!(encoded(&h), "<p>contact [EMAIL] today</p>");
    }

    #[tokio::test]
    async fn attributes_pass_through_untouched() {
        // An email in an attribute is not emitted as an item and survives
        // a round-trip verbatim.
        let raw = r#"<user email="alice@example.com">Bob</user>"#;
        let mut h = load(raw).await;
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str() == "Bob" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[NAME]"));
        h.write_at(rs).await.unwrap();
        assert_eq!(
            encoded(&h),
            r#"<user email="alice@example.com">[NAME]</user>"#
        );
    }
}
