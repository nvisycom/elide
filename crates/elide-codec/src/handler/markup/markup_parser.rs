//! Markup parser: a single streaming pass over an XML (or, leniently, HTML)
//! document into the shared [`ExtractedItem`] stream, recording each item's
//! source byte span so the [`XmlEncoder`] can splice mutated values back
//! **verbatim**.
//!
//! Emits items for element text content, attribute values, comment bodies, and
//! CDATA payloads, each addressed by an exact source span recovered from
//! quick-xml event positions. Each item's `value` is the raw on-the-wire slice
//! (never a decoded form), so encode is a byte-for-byte splice at the recorded
//! span: the declaration, whitespace, tags, and everything outside the redacted
//! spans round-trip unchanged.
//!
//! The same parser serves HTML through a lenient [`MarkupConfig`]: it tolerates
//! HTML's void elements, stray end tags, and bare `&`, and takes the HTML
//! vocabulary (block elements, skipped bodies) as plain element-name lists, so
//! the parser itself knows nothing of `<script>` or "block level".
//!
//! [`ExtractedItem`]: crate::handler::extract::ExtractedItem
//! [`XmlEncoder`]: super::xml_handler::XmlEncoder
//! [`MarkupConfig`]: super::config::MarkupConfig

use std::ops::Range;

use elide_core::modality::Hint;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::{Error, ErrorKind, Result};
use quick_xml::Reader;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use super::config::MarkupConfig;
use super::stream::{MarkupSink, MarkupSource};
use super::xml_handler::XmlItem;
use crate::handler::context::context_words;

/// Extract the redactable markup items from `raw` under `config`. Exposed for
/// the HTML loader (lenient config) and for container formats that run the
/// engine over a part.
pub(crate) fn build_items(raw: &str, config: MarkupConfig<'_>) -> Result<Vec<XmlItem>> {
    MarkupParser::new(raw, config).run()
}

/// One streaming pass over a markup document. Owns the accumulating item
/// stream and the parse state (the open-element stack, the pending
/// text-node/hint records, and any skipped-subtree region), so the per-event
/// handlers are methods sharing `self` rather than a function threading a
/// half-dozen `&mut` locals.
struct MarkupParser<'a> {
    source: MarkupSource<'a>,
    config: MarkupConfig<'a>,
    /// The growing item stream (offset kept in step by `MarkupSink::push`).
    items: MarkupSink,
    /// Text-node records, kept with their engine-space span so a second pass
    /// can attach sibling hints once every text node's span is known.
    text_items: Vec<TextRecord>,
    /// Context hints for non-text items, an attribute value keyed by its
    /// attribute name, a CDATA body by its enclosing element name, applied
    /// alongside the text-node hints at the end.
    extra_hints: Vec<(usize, Vec<Hint<Text>>)>,
    /// The open-element stack: each frame names the element and where its text
    /// children begin, so a block element can group them for sibling hints.
    stack: Vec<Frame>,
    /// A skipped subtree (an HTML `<script>`/`<style>` the caller does not
    /// scan) is opaque: while inside one, all content, text and any
    /// markup-like inner tags, is ignored until the *matching* close, so a
    /// nested element or a stray end tag cannot leak the body or pop early.
    skip: Option<SkipRegion>,
}

impl<'a> MarkupParser<'a> {
    fn new(raw: &'a str, config: MarkupConfig<'a>) -> Self {
        Self {
            source: MarkupSource::new(raw),
            config,
            items: MarkupSink::new(),
            text_items: Vec::new(),
            extra_hints: Vec::new(),
            stack: Vec::new(),
            skip: None,
        }
    }

    fn run(mut self) -> Result<Vec<XmlItem>> {
        let raw = self.source.raw();
        let mut reader = Reader::from_str(raw);
        let cfg = reader.config_mut();
        if self.config.lenient {
            // HTML in the wild is not well-formed: void elements never close,
            // end tags go unmatched, and `&` is often a literal, not an entity.
            cfg.check_end_names = false;
            cfg.allow_unmatched_ends = true;
            cfg.allow_dangling_amp = true;
        }
        // quick-xml reports positions relative to the text *after* a leading
        // BOM, but our spans index the original bytes, so shift past the BOM.
        let bom = bom_len(raw);
        let mut last = bom;

        loop {
            let event = reader.read_event().map_err(malformed)?;
            let span = last..reader.buffer_position() as usize + bom;
            last = span.end;

            if self.skipping(&event) {
                if matches!(event, Event::Eof) {
                    break;
                }
                continue;
            }
            match event {
                Event::Eof => break,
                Event::Start(ref e) => {
                    self.attributes(e)?;
                    self.open_element(e, span);
                }
                Event::Empty(ref e) => self.attributes(e)?,
                Event::End(ref e) => self.close_element(&end_name(e)),
                Event::Text(_) => self.text(span),
                Event::Comment(_) => {
                    if let Some(inner) = strip(span, "<!--", "-->") {
                        self.items
                            .push(self.source.slice(inner.clone()).to_owned(), inner);
                    }
                }
                Event::CData(_) => self.cdata(span),
                _ => {}
            }
        }

        // Any block frames still open at EOF (lenient HTML with unclosed
        // blocks): hint their gathered text too.
        while let Some(frame) = self.stack.pop() {
            if self.config.is_block(&frame.name) {
                attach_sibling_hints(&mut self.text_items[frame.text_start..]);
            }
        }

        self.items.apply_hints(
            self.text_items
                .into_iter()
                .map(|r| (r.item_index, r.pending_hints))
                .chain(self.extra_hints),
        );
        Ok(self.items.into_items())
    }

    /// While inside a skipped subtree, track only that element's own open/close
    /// depth and swallow everything else. Returns whether `event` was consumed
    /// by the skip (the caller should `continue`).
    fn skipping(&mut self, event: &Event<'_>) -> bool {
        let Some(region) = &mut self.skip else {
            return false;
        };
        match event {
            Event::Start(e) if local_name(e) == region.name => region.depth += 1,
            Event::End(e) if end_name(e) == region.name => {
                region.depth -= 1;
                if region.depth == 0 {
                    self.skip = None;
                }
            }
            _ => {}
        }
        true
    }

    /// Push each redactable attribute value as an item hinted by its attribute
    /// name (`ssn="123-45-6789"` → the value is boosted by `ssn`).
    fn attributes(&mut self, e: &BytesStart<'_>) -> Result<()> {
        for (key, inner) in self.source.attributes(e, self.config.lenient)? {
            let idx = self
                .items
                .push(self.source.slice(inner.clone()).to_owned(), inner.clone());
            self.hint_item(
                idx,
                TextLocation::new(inner.start, inner.end),
                &context_words(&key),
            );
        }
        Ok(())
    }

    /// Open an element: enter a skipped subtree, or push a [`Frame`] whose name
    /// becomes the context hint for the element's text and CDATA children.
    fn open_element(&mut self, e: &BytesStart<'_>, span: Range<usize>) {
        let name = local_name(e);
        if self.config.skips_body(&name) {
            self.skip = Some(SkipRegion { name, depth: 1 });
        } else {
            self.stack.push(Frame {
                name,
                hint: hint_words(e),
                tag_span: span,
                text_start: self.text_items.len(),
            });
        }
    }

    /// Close the element named by an end tag. Well-formed XML always closes the
    /// top of the stack, but lenient HTML permits a stray `</div>` with no open
    /// `<div>`: popping unconditionally would then discard the *wrong* frame and
    /// strip the enclosing element-name hint from the following text (which can
    /// change context-gated detection). So match the name against the stack:
    /// ignore an end tag with no open counterpart, and otherwise unwind through
    /// the nearest matching frame, finalizing each block group closed on the way.
    fn close_element(&mut self, name: &str) {
        let Some(depth) = self.stack.iter().rposition(|frame| frame.name == name) else {
            // No matching open element: a stray end tag. Ignore it.
            return;
        };
        // Unwind every frame from the top down to (and including) the match ,
        // an intervening unclosed inline element is closed implicitly here.
        while self.stack.len() > depth {
            let frame = self.stack.pop().expect("frame above the matched depth");
            if self.config.is_block(&frame.name) {
                attach_sibling_hints(&mut self.text_items[frame.text_start..]);
            }
        }
    }

    /// Record a text node, hinted by its enclosing element's name (the markup
    /// counterpart of a JSON key or CSV header vouching for its value:
    /// `<paymentCard>4111…</paymentCard>` lets `paymentCard` boost the number).
    fn text(&mut self, span: Range<usize>) {
        let Some(inner) = self.source.non_blank(span.clone()) else {
            return;
        };
        let value = self.source.slice(inner.clone()).to_owned();
        let engine = self.items.offset()..self.items.offset() + value.len();
        let item_index = self.items.push(value, inner);
        let name_hint = self.enclosing_hint();
        self.text_items.push(TextRecord {
            item_index,
            engine,
            text: self.source.slice(span).trim().to_owned(),
            pending_hints: name_hint.into_iter().collect(),
        });
    }

    /// Record a CDATA body, hinted by its enclosing element name, CDATA is
    /// element content like a text node.
    fn cdata(&mut self, span: Range<usize>) {
        if let Some(inner) = strip(span, "<![CDATA[", "]]>") {
            let idx = self
                .items
                .push(self.source.slice(inner.clone()).to_owned(), inner);
            if let Some(hint) = self.enclosing_hint() {
                self.extra_hints.push((idx, vec![hint]));
            }
        }
    }

    /// The enclosing element's name as a located context hint, if inside one.
    fn enclosing_hint(&self) -> Option<Hint<Text>> {
        self.stack.last().map(|frame| {
            Hint::new(
                TextLocation::new(frame.tag_span.start, frame.tag_span.end),
                TextData::new(frame.hint.clone()),
            )
        })
    }

    /// Record a single located context hint (`text`) on the item at `idx`.
    fn hint_item(&mut self, idx: usize, location: TextLocation, text: &str) {
        self.extra_hints.push((
            idx,
            vec![Hint::new(location, TextData::new(text.to_owned()))],
        ));
    }
}

/// A text node's place in the item stream, its engine-space span, and its
/// trimmed text, the raw material for the sibling-hint pass. `pending_hints`
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
    /// The element name as space-separated words (`paymentCard` → `payment
    /// card`), the text of the context hint attached to the element's content.
    hint: String,
    /// Source span of this element's start tag (`<name …>`), used as the
    /// location of the element-name hint attached to the element's text.
    tag_span: Range<usize>,
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

/// The byte length of a leading UTF-8 BOM (`U+FEFF`), or 0 if absent. quick-xml
/// skips it and reports positions past it, so spans over the original bytes must
/// add it back.
fn bom_len(raw: &str) -> usize {
    if raw.starts_with('\u{feff}') { 3 } else { 0 }
}

/// The start tag's lowercased local name (HTML is case-insensitive; lowercasing
/// makes `<SCRIPT>` and `<P>` match too).
fn local_name(e: &BytesStart<'_>) -> String {
    e.local_name().as_ref().to_ascii_lowercase()
}

/// The start tag's local name as context words for the element's text, see
/// [`context_words`]: `paymentCard` becomes `"payment card"` so a keyword like
/// `card` matches on a word boundary.
///
/// [`context_words`]: crate::handler::context::context_words
fn hint_words(e: &BytesStart<'_>) -> String {
    let local = e.local_name();
    context_words(local.as_ref())
}

/// The end tag's lowercased local name, matched against an open skip element.
fn end_name(e: &BytesEnd<'_>) -> String {
    e.local_name().as_ref().to_ascii_lowercase()
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
/// group, located at that sibling's engine-space span, the surrounding prose a
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
            });
        // Append, don't replace: a text node already carries its enclosing
        // element-name hint, and the sibling prose hints add to it.
        record.pending_hints.extend(hints);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Handler;
    use crate::handler::extract::ExtractHandler;
    use crate::handler::markup::xml_handler::{FORMAT_ID, XmlEncoder, XmlHandler};

    /// Build a handler over `raw` with an explicit config so a test can read
    /// out what the parser extracted (values and hints).
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

    fn xml(raw: &str) -> XmlHandler {
        handler_with(raw, MarkupConfig::xml())
    }

    async fn values(h: &mut XmlHandler) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(chunk) = h.read_next().await.unwrap() {
            out.push(chunk.data.as_str().to_owned());
        }
        out
    }

    #[test]
    fn hint_words_reads_the_local_name_through_context_words() {
        // The splitting rules are exercised in `handler::context`; here we only
        // confirm the element's *local* name (namespace prefix stripped) is what
        // gets tokenized.
        assert_eq!(
            hint_words(&BytesStart::new("c:paymentCard")),
            "payment Card"
        );
    }

    #[tokio::test]
    async fn the_element_name_hints_its_text() {
        // `<paymentCard>` should reach its text as a `payment card` hint, so a
        // context keyword (`card`) can boost the value inside, the markup
        // counterpart of a JSON key or CSV header vouching for its value.
        let raw = "<paymentCard>4111 1111 1111 1111</paymentCard>";
        let mut h = xml(raw);
        let chunk = h.read_next().await.unwrap().expect("one text chunk");
        assert_eq!(chunk.data.as_str(), "4111 1111 1111 1111");
        assert!(
            chunk
                .hints
                .iter()
                .any(|hint| hint.data.as_str() == "payment Card"),
            "expected a `payment Card` element-name hint, got {:?}",
            chunk
                .hints
                .iter()
                .map(|h| h.data.as_str())
                .collect::<Vec<_>>(),
        );
    }

    #[tokio::test]
    async fn a_leading_bom_does_not_shift_extracted_text() {
        // quick-xml reports positions past a leading BOM; the extractor must add
        // it back so the streamed text is exact (not shifted into garbage) and a
        // recognizer can match it.
        let raw = "\u{feff}<?xml version=\"1.0\"?><r>alice@example.com</r>";
        let mut h = xml(raw);
        let vs = values(&mut h).await;
        assert!(
            vs.iter().any(|v| v == "alice@example.com"),
            "BOM shifted the extracted text: {vs:?}"
        );
    }

    #[tokio::test]
    async fn stream_yields_text_comment_cdata_and_attributes() {
        // The four redactable item kinds our parser surfaces from one document:
        // element text, an attribute value, a comment body, and a CDATA payload.
        let raw =
            r#"<root id="A1"><name>Alice</name><!-- c --><data><![CDATA[secret]]></data></root>"#;
        let mut h = xml(raw);
        let vs = values(&mut h).await;
        assert!(vs.iter().any(|v| v == "Alice"), "text: {vs:?}");
        assert!(vs.iter().any(|v| v == " c "), "comment: {vs:?}");
        assert!(vs.iter().any(|v| v == "secret"), "cdata: {vs:?}");
        assert!(vs.iter().any(|v| v == "A1"), "attr: {vs:?}");
    }

    /// A small synthetic block/skip vocabulary. The real HTML vocabulary is
    /// exercised by the HTML loader's own tests; here it is a stand-in so the
    /// test targets *our* skip-region tracking, not the HTML element lists.
    #[cfg(feature = "html")]
    const TEST_BLOCKS: &[&str] = &["p", "div"];

    #[cfg(feature = "html")]
    #[tokio::test]
    async fn skipped_body_stays_opaque_through_nested_and_stray_markup() {
        // Our skip-region depth tracking: a skipped body is opaque, so nested
        // markup-like tags and a stray end tag inside it must not leak its text
        // or pop the skip early. (The plain skip-vs-scan case is the HTML
        // loader's, over the real vocabulary; this pins the depth edge cases.)
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
    async fn a_stray_end_tag_does_not_pop_the_wrong_frame() {
        // Lenient parsing emits an `End(div)` for a `</div>` with no open
        // `<div>`. Popping unconditionally would discard the still-open
        // `<paymentCard>` frame, stripping the element-name hint from the text
        // that follows, which can change context-gated detection. The stray
        // end tag must be ignored so the following number keeps its hint.
        let raw = "<paymentCard>4111 <b>1111</b></div> 1111 1111</paymentCard>";
        let mut h = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &[]));
        let mut hinted = false;
        while let Some(chunk) = h.read_next().await.unwrap() {
            // The text after the stray `</div>` must still carry the
            // `payment Card` element-name hint.
            if chunk.data.as_str().contains("1111 1111")
                && chunk
                    .hints
                    .iter()
                    .any(|hint| hint.data.as_str() == "payment Card")
            {
                hinted = true;
            }
        }
        assert!(
            hinted,
            "text after a stray </div> lost its enclosing paymentCard hint"
        );
    }
}
