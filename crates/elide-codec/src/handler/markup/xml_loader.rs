//! Markup loader: parses XML (and, leniently, HTML) into the shared
//! [`ExtractedItem`] stream, recording each item's source byte span so the
//! [`XmlEncoder`] can splice mutated values back **verbatim**.
//!
//! Emits items for element text content, attribute values, comment bodies, and
//! CDATA payloads, each addressed by an exact source span recovered from
//! quick-xml event positions. Each item's `value` is the raw on-the-wire slice
//! (never a decoded form), so encode is a byte-for-byte splice at the recorded
//! span: the declaration, whitespace, tags, and everything outside the redacted
//! spans round-trip unchanged.
//!
//! The same engine serves HTML through a lenient [`MarkupConfig`]: it tolerates
//! HTML's void elements, stray end tags, and bare `&`, and takes the HTML
//! vocabulary (block elements, skipped bodies) as plain element-name lists, so
//! the engine itself knows nothing of `<script>` or "block level".
//!
//! [`ExtractedItem`]: super::ExtractedItem
//! [`XmlEncoder`]: super::XmlEncoder
//! [`MarkupConfig`]: super::config::MarkupConfig

use std::borrow::Cow;
use std::ops::Range;

use elide_core::modality::Hint;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::{Error, ErrorKind, Result};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::config::MarkupConfig;
use super::xml_handler::{FORMAT_ID, XmlEncoder, XmlHandler, XmlItem, XmlSpan};
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::{ExtractHandler, ExtractedItem};

/// Loader for XML files. Produces one [`XmlHandler`] per input.
#[derive(Debug)]
pub(crate) struct XmlLoader;

#[async_trait::async_trait]
impl Loader<Text> for XmlLoader {
    type Handler = XmlHandler;

    async fn decode(&self, content: ContentData) -> Result<XmlHandler> {
        let text = content.decode()?;
        let items = build_items(&text, MarkupConfig::xml())?;
        Ok(ExtractHandler::new(
            FORMAT_ID.clone(),
            XmlEncoder { raw: text },
            items,
        ))
    }
}

/// Extract the redactable markup items from `raw` under `config`. Exposed for
/// the HTML loader (lenient config) and for container formats that run the
/// engine over a part.
pub(crate) fn build_items(raw: &str, config: MarkupConfig<'_>) -> Result<Vec<XmlItem>> {
    let mut reader = Reader::from_str(raw);
    let cfg = reader.config_mut();
    if config.lenient {
        // HTML in the wild is not well-formed: void elements never close,
        // end tags go unmatched, and `&` is often a literal, not an entity.
        cfg.check_end_names = false;
        cfg.allow_unmatched_ends = true;
        cfg.allow_dangling_amp = true;
    }

    let mut items = Vec::new();
    // Text-node items, recorded with their engine-space span so a second pass
    // can attach sibling hints once every text node's span is known.
    let mut text_items: Vec<TextRecord> = Vec::new();
    // The open-element stack: each frame is (lowercased name, index into
    // `text_items` where this element's text children begin), so we can group a
    // text node with the others under its nearest block ancestor.
    let mut stack: Vec<Frame> = Vec::new();
    let mut last = 0usize;
    // Running engine-space offset (the cumulative length of item values), the
    // coordinate a chunk carries and a hint resolves against.
    let mut offset = 0usize;

    loop {
        let event = reader.read_event().map_err(malformed)?;
        let span = last..reader.buffer_position() as usize;
        last = span.end;

        match event {
            Event::Eof => break,
            Event::Start(ref e) => {
                emit_attributes(raw, e, &mut items, &mut offset);
                let name = local_name(e);
                stack.push(Frame {
                    skip_body: config.skips_body(&name),
                    name,
                    text_start: text_items.len(),
                });
            }
            Event::Empty(ref e) => {
                // A self-closing element: attributes only, no body, no frame.
                emit_attributes(raw, e, &mut items, &mut offset);
            }
            Event::End(_) => {
                if let Some(frame) = stack.pop() {
                    // A block element closes: the text nodes gathered since it
                    // opened share its context, so hint them against each other.
                    if config.is_block(&frame.name) {
                        attach_sibling_hints(&mut text_items[frame.text_start..]);
                    }
                }
            }
            Event::Text(_) => {
                if let Some(inner) = non_blank(raw, span.clone()) {
                    // Inside a skipped body (an HTML `<script>`/`<style>` the
                    // caller does not scan), emit nothing: the text is neither
                    // an item nor part of the engine-space stream.
                    if in_skipped_body(&stack) {
                        continue;
                    }
                    let value = raw[inner.clone()].to_owned();
                    let engine = offset..offset + value.len();
                    offset = engine.end;
                    let item_index = items.len();
                    items.push(span_item(value, inner));
                    text_items.push(TextRecord {
                        item_index,
                        engine,
                        text: raw[span].trim().to_owned(),
                        pending_hints: Vec::new(),
                    });
                }
            }
            Event::Comment(_) => {
                if let Some(inner) = strip(span, "<!--", "-->") {
                    push_span(raw, inner, &mut items, &mut offset);
                }
            }
            Event::CData(_) => {
                if let Some(inner) = strip(span, "<![CDATA[", "]]>") {
                    push_span(raw, inner, &mut items, &mut offset);
                }
            }
            _ => {}
        }
    }

    // Any block frames still open at EOF (lenient HTML with unclosed blocks):
    // hint their gathered text too.
    while let Some(frame) = stack.pop() {
        if config.is_block(&frame.name) {
            attach_sibling_hints(&mut text_items[frame.text_start..]);
        }
    }

    apply_text_hints(&mut items, &text_items);
    Ok(items)
}

/// A text node's place in the item stream, its engine-space span, and its
/// trimmed text — the raw material for the sibling-hint pass. `pending_hints`
/// is filled when its block group closes and moved onto the item at the end.
struct TextRecord {
    item_index: usize,
    engine: Range<usize>,
    text: String,
    pending_hints: Vec<Hint<Text>>,
}

/// An open element on the parse stack.
struct Frame {
    name: String,
    text_start: usize,
    /// Whether this element's body text is skipped (not emitted).
    skip_body: bool,
}

/// Whether the innermost open element's body is skipped, so its text must not
/// enter the stream.
fn in_skipped_body(stack: &[Frame]) -> bool {
    matches!(stack.last(), Some(f) if f.skip_body)
}

fn malformed(e: quick_xml::Error) -> Error {
    Error::new(ErrorKind::MalformedInput, format!("malformed markup: {e}"))
}

/// The element's lowercased local name (HTML is case-insensitive; lowercasing
/// makes `<SCRIPT>` and `<P>` match too).
fn local_name(e: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase()
}

/// Emit one item per attribute value with a text-bearing value, addressed by the
/// value's inner byte span (between the quotes) in the source. Values pass
/// through verbatim, so a `mailto:` URL has its email matched in place.
fn emit_attributes(raw: &str, e: &BytesStart<'_>, items: &mut Vec<XmlItem>, offset: &mut usize) {
    for attr in e.attributes().with_checks(false).flatten() {
        // quick-xml borrows an unescaped value directly out of the source
        // buffer, so its slice position *is* the source span. A value with
        // entities decodes to an owned buffer with no source position; such
        // attributes are left un-redactable (rare, and never a plain-text PII
        // carrier) rather than guessed at.
        let Cow::Borrowed(bytes) = attr.value else {
            continue;
        };
        let Some(inner) = slice_span(raw, bytes) else {
            continue;
        };
        if raw[inner.clone()].trim().is_empty() {
            continue;
        }
        push_span(raw, inner, items, offset);
    }
}

/// The byte range that `slice` — a subslice borrowed out of `raw` — occupies in
/// `raw`, or `None` if it is not a valid in-bounds char-aligned subslice.
fn slice_span(raw: &str, slice: &[u8]) -> Option<Range<usize>> {
    let base = raw.as_ptr() as usize;
    let start = (slice.as_ptr() as usize).checked_sub(base)?;
    let end = start.checked_add(slice.len())?;
    (end <= raw.len() && raw.is_char_boundary(start) && raw.is_char_boundary(end))
        .then_some(start..end)
}

/// Push an item for the verbatim source slice at `inner`, advancing the
/// engine-space offset.
fn push_span(raw: &str, inner: Range<usize>, items: &mut Vec<XmlItem>, offset: &mut usize) {
    let value = raw[inner.clone()].to_owned();
    *offset += value.len();
    items.push(span_item(value, inner));
}

/// Return `span` unless it covers only whitespace.
fn non_blank(raw: &str, span: Range<usize>) -> Option<Range<usize>> {
    (!raw[span.clone()].trim().is_empty()).then_some(span)
}

/// Build an item whose value is `value`, addressed by source `span`.
fn span_item(value: String, span: Range<usize>) -> XmlItem {
    ExtractedItem {
        value,
        address: XmlSpan(span),
        hints: Vec::new(),
    }
}

/// Narrow `span` by its `open`/`close` delimiters, returning the inner range.
fn strip(span: Range<usize>, open: &str, close: &str) -> Option<Range<usize>> {
    let start = span.start.checked_add(open.len())?;
    let end = span.end.checked_sub(close.len())?;
    (start <= end).then_some(start..end)
}

/// Give each text record in a block group one hint per *other* record in the
/// group, located at that sibling's engine-space span — the surrounding prose a
/// context boost points back at when a sentence is split across inline wrappers.
fn attach_sibling_hints(group: &mut [TextRecord]) {
    let siblings: Vec<(Range<usize>, String)> = group
        .iter()
        .filter(|r| !r.text.is_empty())
        .map(|r| (r.engine.clone(), r.text.clone()))
        .collect();
    if siblings.len() < 2 {
        return;
    }
    for record in group.iter_mut() {
        let hints = siblings
            .iter()
            .filter(|(engine, _)| *engine != record.engine)
            .map(|(engine, text)| {
                Hint::new(
                    TextLocation::new(engine.start, engine.end),
                    TextData::new(text.clone()),
                )
            })
            .collect::<Vec<_>>();
        if !hints.is_empty() {
            record.pending_hints = hints;
        }
    }
}

/// Move each text record's accumulated hints onto its item.
fn apply_text_hints(items: &mut [XmlItem], text_items: &[TextRecord]) {
    for record in text_items {
        if !record.pending_hints.is_empty() {
            items[record.item_index].hints = record.pending_hints.clone();
        }
    }
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

    /// Build a handler over `raw` with an explicit config (for the HTML paths).
    fn handler_with(raw: &str, config: MarkupConfig) -> XmlHandler {
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

    async fn values(h: &mut XmlHandler) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(chunk) = h.read_next().await.unwrap() {
            out.push(chunk.data.as_str().to_owned());
        }
        out
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
    async fn stream_yields_text_comment_cdata_and_attributes() {
        let raw =
            r#"<root id="A1"><name>Alice</name><!-- c --><data><![CDATA[secret]]></data></root>"#;
        let mut h = load(raw).await;
        let vs = values(&mut h).await;
        assert!(vs.iter().any(|v| v == "Alice"), "text: {vs:?}");
        assert!(vs.iter().any(|v| v == " c "), "comment: {vs:?}");
        assert!(vs.iter().any(|v| v == "secret"), "cdata: {vs:?}");
        // Attribute values are now redactable, so `A1` is emitted.
        assert!(vs.iter().any(|v| v == "A1"), "attr: {vs:?}");
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
            chunk.location.start + at,
            chunk.location.start + at + "alice@example.com".len(),
        );
        let mut rs = Redactions::new();
        rs.push(loc, TextReplacement::substituted("[EMAIL]"));
        h.write_at(rs).await.unwrap();
        assert_eq!(encoded(&h), "<p>contact [EMAIL] today</p>");
    }

    // --- lenient (HTML-style) paths over the same engine -----------------

    /// A small block vocabulary for the engine's lenient-mode tests; the real
    /// HTML block set is exercised in the HTML loader's own tests.
    const TEST_BLOCKS: &[&str] = &["p", "div"];

    fn html(raw: &str) -> XmlHandler {
        handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &[]))
    }

    #[tokio::test]
    async fn html_tolerates_bare_ampersand_and_round_trips() {
        for raw in [
            "<p>Q&A here</p>",
            r#"<a href="/x?a=1&b=2">link</a>"#,
            "<p>hi</div>",
            "<p>one<p>two<br>three",
            "<!DOCTYPE html><p>a@b.com</p>",
        ] {
            let h = html(raw);
            assert_eq!(encoded(&h), raw, "html round-trip changed: {raw:?}");
        }
    }

    #[tokio::test]
    async fn html_redacts_text_and_attribute() {
        let raw = r#"<html><body><img alt="alice@example.com"><p>Bob</p></body></html>"#;
        let mut h = html(raw);
        let vs = values(&mut h).await;
        assert!(
            vs.iter().any(|v| v == "alice@example.com"),
            "alt attr: {vs:?}"
        );
        assert!(vs.iter().any(|v| v == "Bob"), "text: {vs:?}");

        let mut h = html(raw);
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str() == "alice@example.com" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[email]"));
        h.write_at(rs).await.unwrap();
        assert!(
            encoded(&h).contains(r#"alt="[email]""#),
            "alt not rewritten: {}",
            encoded(&h)
        );
    }

    #[tokio::test]
    async fn skipped_body_vs_scanned_body() {
        let raw = r#"<html><body><script>var a="alice@example.com";</script></body></html>"#;

        // `script` in skip_body_elements: the body never enters the stream.
        let mut skip = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &["script"]));
        let vs = values(&mut skip).await;
        assert!(
            !vs.iter().any(|v| v.contains("alice@example.com")),
            "skipped body leaked: {vs:?}"
        );

        // Not listed: the body is scanned as ordinary text.
        let mut scan = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &[]));
        let vs = values(&mut scan).await;
        assert!(
            vs.iter().any(|v| v.contains("alice@example.com")),
            "scanned body missed: {vs:?}"
        );
    }

    #[tokio::test]
    async fn html_sibling_hints_across_inline_wrapper() {
        // A card number split by an inline <code> wrapper: each text chunk
        // should carry the neighbouring prose as a located hint.
        let raw = "<p>Card <code>4111 1111 1111 1111</code> on file</p>";
        let mut h = html(raw);
        let mut any_hint = false;
        while let Some(chunk) = h.read_next().await.unwrap() {
            if !chunk.hints.is_empty() {
                any_hint = true;
            }
        }
        assert!(any_hint, "expected sibling hints across the inline wrapper");
    }
}
