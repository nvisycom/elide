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

use std::ops::Range;

use elide_core::modality::Hint;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::{Error, ErrorKind, Result};
use quick_xml::Reader;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use super::config::MarkupConfig;
use super::stream::{MarkupSink, MarkupSource};
use super::xml_handler::{FORMAT_ID, XmlEncoder, XmlHandler, XmlItem};
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::ExtractHandler;

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
    let source = MarkupSource::new(raw);
    let mut reader = Reader::from_str(raw);
    let cfg = reader.config_mut();
    if config.lenient {
        // HTML in the wild is not well-formed: void elements never close,
        // end tags go unmatched, and `&` is often a literal, not an entity.
        cfg.check_end_names = false;
        cfg.allow_unmatched_ends = true;
        cfg.allow_dangling_amp = true;
    }

    // The growing item stream (offset kept in step by `Items::push`).
    let mut items = MarkupSink::new();
    // Text-node records, kept with their engine-space span so a second pass can
    // attach sibling hints once every text node's span is known.
    let mut text_items: Vec<TextRecord> = Vec::new();
    // The open-element stack: each frame names the element and where its text
    // children begin, so a block element can group them for sibling hints.
    let mut stack: Vec<Frame> = Vec::new();
    // A skipped subtree (an HTML `<script>`/`<style>` the caller does not scan)
    // is opaque: while inside one, all content — text and any markup-like inner
    // tags — is ignored until the *matching* close, so nested elements or a
    // stray end tag cannot leak the body or pop the skip early.
    let mut skip: Option<SkipRegion> = None;
    let mut last = 0usize;

    loop {
        let event = reader.read_event().map_err(malformed)?;
        let span = last..reader.buffer_position() as usize;
        last = span.end;

        // Inside a skipped subtree, only the skip element's *own* open/close
        // depth is tracked; every other event — text, nested tags, a stray end
        // tag for some other element — is opaque, so it cannot leak the body or
        // pop the skip early.
        if let Some(region) = &mut skip {
            match event {
                Event::Eof => break,
                Event::Start(ref e) if local_name(e) == region.name => region.depth += 1,
                Event::End(ref e) if end_name(e) == region.name => {
                    region.depth -= 1;
                    if region.depth == 0 {
                        skip = None;
                    }
                }
                _ => {}
            }
            continue;
        }

        match event {
            Event::Eof => break,
            Event::Start(ref e) => {
                for inner in source.attribute_spans(e, config.lenient)? {
                    items.push(source.slice(inner.clone()).to_owned(), inner);
                }
                let name = local_name(e);
                if config.skips_body(&name) {
                    skip = Some(SkipRegion { name, depth: 1 });
                } else {
                    stack.push(Frame {
                        name,
                        text_start: text_items.len(),
                    });
                }
            }
            Event::Empty(ref e) => {
                // A self-closing element: attributes only, no body, no frame.
                for inner in source.attribute_spans(e, config.lenient)? {
                    items.push(source.slice(inner.clone()).to_owned(), inner);
                }
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
                if let Some(inner) = source.non_blank(span.clone()) {
                    let value = source.slice(inner.clone()).to_owned();
                    let engine = items.offset()..items.offset() + value.len();
                    let item_index = items.push(value, inner);
                    text_items.push(TextRecord {
                        item_index,
                        engine,
                        text: source.slice(span).trim().to_owned(),
                        pending_hints: Vec::new(),
                    });
                }
            }
            Event::Comment(_) => {
                if let Some(inner) = strip(span, "<!--", "-->") {
                    items.push(source.slice(inner.clone()).to_owned(), inner);
                }
            }
            Event::CData(_) => {
                if let Some(inner) = strip(span, "<![CDATA[", "]]>") {
                    items.push(source.slice(inner.clone()).to_owned(), inner);
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

    items.apply_hints(
        text_items
            .into_iter()
            .map(|r| (r.item_index, r.pending_hints)),
    );
    Ok(items.into_items())
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
}

/// The active skipped subtree: the element whose body is opaque, and the open
/// depth of that same element name, so a nested `<script>` inside a `<script>`
/// (or a stray close) resolves to the matching close.
struct SkipRegion {
    name: String,
    depth: usize,
}

fn malformed(e: quick_xml::Error) -> Error {
    Error::new(ErrorKind::MalformedInput, format!("malformed markup: {e}"))
}

/// The start tag's lowercased local name (HTML is case-insensitive; lowercasing
/// makes `<SCRIPT>` and `<P>` match too).
fn local_name(e: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase()
}

/// The end tag's lowercased local name, matched against an open skip element.
fn end_name(e: &BytesEnd<'_>) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase()
}

/// Narrow `span` by its `open`/`close` delimiters, returning the inner range.
fn strip(span: Range<usize>, open: &str, close: &str) -> Option<Range<usize>> {
    let start = span.start.checked_add(open.len())?;
    let end = span.end.checked_sub(close.len())?;
    (start <= end).then_some(start..end)
}

/// Cap on block-group size for sibling hints. The fan-out is one hint per pair
/// of records, so a pathological block with thousands of text runs would emit
/// millions of hints. Context boosting adds little once a group is this large
/// (the sentence-fragment case it targets has a handful of runs), so a bigger
/// group is left un-hinted rather than risk quadratic blow-up.
const MAX_SIBLING_HINT_GROUP: usize = 64;

/// Give each text record in a block group one hint per *other* record in the
/// group, located at that sibling's engine-space span — the surrounding prose a
/// context boost points back at when a sentence is split across inline wrappers.
fn attach_sibling_hints(group: &mut [TextRecord]) {
    let siblings: Vec<(Range<usize>, String)> = group
        .iter()
        .filter(|r| !r.text.is_empty())
        .map(|r| (r.engine.clone(), r.text.clone()))
        .collect();
    if siblings.len() < 2 || siblings.len() > MAX_SIBLING_HINT_GROUP {
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

    /// Build a handler over `raw` with an explicit config (for the lenient
    /// HTML-style paths).
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
    async fn strict_xml_rejects_a_duplicate_attribute() {
        // A duplicate attribute key is malformed XML; strict parsing refuses it.
        let raw = r#"<root id="a" id="b"/>"#;
        let err = XmlLoader
            .decode(ContentData::from_text(raw))
            .await
            .expect_err("duplicate attribute must be rejected");
        assert_eq!(err.kind(), elide_core::ErrorKind::MalformedInput);
    }

    #[tokio::test]
    async fn lift_carries_the_exact_source_span() {
        use elide_core::modality::text::SourceRef;

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

    // Lenient (HTML-style) paths over the same engine. Gated on `html`: the
    // lenient config only exists when HTML is compiled.

    /// A small block vocabulary for the engine's lenient-mode tests; the real
    /// HTML block set is exercised in the HTML loader's own tests.
    #[cfg(feature = "html")]
    const TEST_BLOCKS: &[&str] = &["p", "div"];

    #[cfg(feature = "html")]
    fn html(raw: &str) -> XmlHandler {
        handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &[]))
    }

    #[cfg(feature = "html")]
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

    #[cfg(feature = "html")]
    #[tokio::test]
    async fn lenient_html_tolerates_a_duplicate_attribute() {
        // What strict XML rejects, lenient HTML accepts and round-trips.
        let raw = r#"<img id="a" id="b">"#;
        let h = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &[]));
        assert_eq!(encoded(&h), raw);
    }

    #[cfg(feature = "html")]
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

    #[cfg(feature = "html")]
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

    #[cfg(feature = "html")]
    #[tokio::test]
    async fn skipped_body_stays_opaque_through_nested_and_stray_markup() {
        // A skipped body is opaque: markup-like inner tags and a stray end tag
        // inside it must not leak its text or pop the skip early.
        let cases = [
            // Nested element inside the skipped script.
            r#"<p>ok</p><script>var a; <b>alice@example.com</b> more;</script><p>after</p>"#,
            // Stray end tag inside the skipped script.
            r#"<p>ok</p><script>a; </div> alice@example.com;</script><p>after</p>"#,
            // Nested same-name element: the inner </script> must not end the skip
            // before the outer content is consumed.
            r#"<p>ok</p><script>x<script>alice@example.com</script>y</script><p>after</p>"#,
        ];
        for raw in cases {
            let mut h = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &["script"]));
            let vs = values(&mut h).await;
            assert!(
                !vs.iter().any(|v| v.contains("alice@example.com")),
                "skipped body leaked for {raw:?}: {vs:?}"
            );
            // The surrounding text is still seen and the skip closed cleanly.
            assert!(vs.iter().any(|v| v == "ok"), "pre-text missing: {raw:?}");
            assert!(
                vs.iter().any(|v| v == "after"),
                "post-text missing: {raw:?}"
            );
        }
    }

    #[cfg(feature = "html")]
    #[tokio::test]
    async fn redaction_after_a_skipped_script_is_byte_faithful() {
        // Redacting text that follows a skipped script leaves the script and all
        // other markup byte-identical, changing only the targeted span.
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

    #[cfg(feature = "html")]
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
