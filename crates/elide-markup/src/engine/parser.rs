//! Markup parser: a single streaming pass over an XML (or, leniently, HTML)
//! document into the shared [`ExtractedItem`] stream, recording each item's
//! source byte span so the [`MarkupRecombine`] can splice mutated values back
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
//! [`ExtractedItem`]: elide_codec::extract::ExtractedItem
//! [`MarkupRecombine`]: super::recombine::MarkupRecombine
//! [`MarkupConfig`]: super::config::MarkupConfig

use std::ops::Range;

use elide_codec::extract::{ItemFlag, Passage};
use elide_codec::string::ContextWords;
use elide_core::entity::builtins;
use elide_core::modality::ResolvedHint;
use elide_core::modality::text::{SourceRef, Text, TextData, TextLocation};
use elide_core::redaction::HiddenContentKind;
use elide_core::{Error, Result};
use quick_xml::Reader;
use quick_xml::events::{BytesEnd, BytesStart, Event};

use super::address::{SpanKind, XmlItem, XmlSpan};
use super::config::MarkupConfig;
use super::meta::MetaTag;
use super::source::{AttributeSpan, MarkupSink, MarkupSource, StartTag};
use super::tag::HtmlTagExt;

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
    extra_hints: Vec<(usize, Vec<ResolvedHint<Text>>)>,
    /// The open-element stack: each frame names the element and where its text
    /// children begin, so a block element can group them for sibling hints.
    stack: Vec<Frame>,
    /// A skipped subtree (an HTML `<script>`/`<style>` the caller does not
    /// scan) is opaque: while inside one, all content, text and any
    /// markup-like inner tags, is ignored until the *matching* close, so a
    /// nested element or a stray end tag cannot leak the body or pop early.
    skip: Option<SkipRegion>,
    /// The pending text run: its text and character reference events, which
    /// read as one value.
    run: Option<Range<usize>>,
    /// Whether the last text pushed opened a passage that is still open.
    in_passage: bool,
    /// How many passages text has started, numbering each text record's.
    passages: usize,
    /// The attribute values and comments met inside the open passage, pushed
    /// once it ends so the passage's items stay contiguous.
    deferred: Vec<Deferred>,
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
            run: None,
            in_passage: false,
            passages: 0,
            deferred: Vec::new(),
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
            let event = reader
                .read_event()
                .map_err(|e| Error::malformed(format!("malformed markup: {e}")))?;
            let span = last..reader.buffer_position() as usize + bom;
            last = span.end;

            if self.skipping(&event)? {
                if matches!(event, Event::Eof) {
                    break;
                }
                continue;
            }
            // quick-xml splits a text run at each character reference; the
            // run is read whole once any other event ends it.
            if let Event::Text(_) | Event::GeneralRef(_) = event {
                self.run = Some(
                    self.run
                        .take()
                        .map_or(span.clone(), |run| run.start..span.end),
                );
                continue;
            }
            if let Some(run) = self.run.take() {
                self.text(run);
            }
            match event {
                Event::Eof => break,
                Event::Start(ref e) => {
                    let mut tag = self.source.start_tag(e, self.config.lenient)?;
                    self.flag_meta(&mut tag);
                    self.open_passage_element(&tag.name, span.clone());
                    self.attributes(tag.attributes);
                    self.open_element(tag.name, &tag.written, span);
                }
                Event::Empty(ref e) => {
                    let mut tag = self.source.start_tag(e, self.config.lenient)?;
                    self.flag_meta(&mut tag);
                    self.open_passage_element(&tag.name, span);
                    self.attributes(tag.attributes);
                }
                Event::End(ref e) => {
                    let name = end_name(e, self.source.raw(), self.config.lenient)?;
                    if self.config.bounds_passage(&name) {
                        self.end_passage();
                    }
                    self.close_element(&name);
                }
                Event::Comment(_) => {
                    if let Some(inner) = strip(span, "<!--", "-->") {
                        if self.in_passage {
                            self.deferred.push(Deferred::Comment(inner));
                        } else {
                            self.comment(inner);
                        }
                    }
                }
                Event::CData(_) => self.cdata(span),
                _ => {}
            }
        }
        self.end_passage();

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
    fn skipping(&mut self, event: &Event<'_>) -> Result<bool> {
        let (raw, html) = (self.source.raw(), self.config.lenient);
        let Some(region) = &mut self.skip else {
            return Ok(false);
        };
        match event {
            Event::Start(e) if start_name(e, raw, html)? == region.name => region.depth += 1,
            Event::End(e) if end_name(e, raw, html)? == region.name => {
                region.depth -= 1;
                if region.depth == 0 {
                    self.skip = None;
                }
            }
            _ => {}
        }
        Ok(true)
    }

    /// Track the passage at the element `name` opening at `span`: a line break
    /// reads on in it as a separator, and an element that bounds a passage ends
    /// it.
    fn open_passage_element(&mut self, name: &str, span: Range<usize>) {
        if self.config.is_line_break(name) {
            if self.in_passage {
                self.items
                    .push(XmlSpan::separator(span, "\n"), Passage::Separator);
            }
        } else if self.config.bounds_passage(name) {
            self.end_passage();
        }
    }

    /// End the open passage and push what was deferred inside it.
    fn end_passage(&mut self) {
        self.in_passage = false;
        for deferred in std::mem::take(&mut self.deferred) {
            match deferred {
                Deferred::Attribute(attr) => self.attribute(attr),
                Deferred::Comment(inner) => self.comment(inner),
            }
        }
    }

    /// Push each redactable attribute value, or defer it while a passage is
    /// open.
    fn attributes(&mut self, attributes: Vec<AttributeSpan>) {
        for attr in attributes {
            if self.in_passage {
                self.deferred.push(Deferred::Attribute(attr));
            } else {
                self.attribute(attr);
            }
        }
    }

    /// Push a comment body, flagged as what a collaborator wrote.
    fn comment(&mut self, inner: Range<usize>) {
        let index = self
            .items
            .push(XmlSpan::verbatim(self.source.raw(), inner), Passage::Start);
        let flag = ItemFlag::new(
            builtins::COMMUNICATIONS_CONTENT.to_ref(),
            HiddenContentKind::Collaboration,
        );
        self.items.flag(index, flag);
    }

    /// Flag every `content` of an HTML `<meta>` field as the metadata it is.
    fn flag_meta(&self, tag: &mut StartTag) {
        let Some(label) = tag.meta.as_ref().and_then(MetaTag::label) else {
            return;
        };
        for attr in &mut tag.attributes {
            let is_content = attr
                .name
                .clone()
                .is_some_and(|name| self.source.slice(name).eq_ignore_ascii_case("content"));
            if is_content {
                attr.flag = Some(ItemFlag::new(label.clone(), HiddenContentKind::Metadata));
            }
        }
    }

    /// Push a redactable attribute value as an item hinted by its attribute
    /// name (`ssn="123-45-6789"` → the value is boosted by `ssn`).
    fn attribute(&mut self, attr: AttributeSpan) {
        let value = self.escaped(attr.value, SpanKind::Attribute);
        let idx = self.items.push(value, Passage::Start);
        if let Some(flag) = attr.flag {
            self.items.flag(idx, flag);
        }
        // Hint the value with its attribute name's location and context
        // words, when the name was placed in the source (the out-of-band
        // context that boosts it). Strip any namespace prefix (`ns:attr`)
        // so the hint keys on the local name, as the element path does.
        if let Some(name) = attr.name {
            let raw = self.source.slice(name.clone());
            let local = raw.rsplit(':').next().unwrap_or(raw);
            // The attribute name is a raw *source* span, not a decoded one, so
            // it is a source-only location; a consumer must resolve it against
            // the source bytes, never the extracted value.
            let location = TextLocation::marked_source(SourceRef::new(name));
            self.hint_item(idx, location, local.context_words());
        }
    }

    /// Open the element `name`, `written` so in the source: enter a skipped
    /// subtree, or push a [`Frame`] whose name becomes the context hint for the
    /// element's text and CDATA children.
    fn open_element(&mut self, name: String, written: &str, span: Range<usize>) {
        if self.config.skips_body(&name) {
            self.skip = Some(SkipRegion { name, depth: 1 });
        } else {
            // The hint text is the element name split into context words,
            // computed from the name *as written* so a camelCase boundary
            // (`paymentCard` → `payment Card`) survives; `name` itself is
            // lowercased for tag matching and would lose that boundary.
            let hint = written.context_words();
            self.stack.push(Frame {
                name,
                hint,
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
    ///
    /// Text inside an open passage reads on in it, a whitespace-only node too
    /// (the space between two inline elements); whitespace-only text opens no
    /// passage.
    fn text(&mut self, span: Range<usize>) {
        let (value, address) = self.escaped(span, SpanKind::Text);
        if value.trim().is_empty() {
            if self.in_passage {
                self.items.push((value, address), Passage::Continue);
            }
            return;
        }
        let passage = if self.in_passage {
            Passage::Continue
        } else {
            self.passages += 1;
            Passage::Start
        };
        self.in_passage = self.config.inline_passages;
        let engine = self.items.offset()..self.items.offset() + value.len();
        let text = value.trim().to_owned();
        let item_index = self.items.push((value, address), passage);
        let name_hint = self.enclosing_hint();
        self.text_items.push(TextRecord {
            item_index,
            engine,
            passage: self.passages,
            text,
            pending_hints: name_hint.into_iter().collect(),
        });
    }

    /// The decoded value of the escaped `kind` text at `range`, and its
    /// address.
    fn escaped(&self, range: Range<usize>, kind: SpanKind) -> (String, XmlSpan) {
        XmlSpan::escaped(self.source.raw(), range, kind, self.config.references)
    }

    /// Record a CDATA body, hinted by its enclosing element name, CDATA is
    /// element content like a text node.
    fn cdata(&mut self, span: Range<usize>) {
        self.end_passage();
        if let Some(inner) = strip(span, "<![CDATA[", "]]>") {
            let idx = self
                .items
                .push(XmlSpan::verbatim(self.source.raw(), inner), Passage::Start);
            if let Some(hint) = self.enclosing_hint() {
                self.extra_hints.push((idx, vec![hint]));
            }
        }
    }

    /// The enclosing element's name as a located context hint, if inside one:
    /// located at the start tag, carrying the element name split into context
    /// words (`paymentCard` → `payment Card`) so a keyword matches on a word
    /// boundary.
    fn enclosing_hint(&self) -> Option<ResolvedHint<Text>> {
        self.stack.last().map(|frame| {
            // The start-tag span is a raw *source* range, so the hint is a
            // source-only location, resolved against the source, not the value.
            let location = TextLocation::marked_source(SourceRef::new(frame.tag_span.clone()));
            ResolvedHint::new(location, TextData::new(frame.hint.clone()))
        })
    }

    /// Record a located context hint on the item at `idx`, carrying `text` (the
    /// context words of the labelling name) at `location`.
    fn hint_item(&mut self, idx: usize, location: TextLocation, text: String) {
        self.extra_hints
            .push((idx, vec![ResolvedHint::new(location, TextData::new(text))]));
    }
}

/// A text node's place in the item stream, its engine-space span, and its
/// trimmed text, the raw material for the sibling-hint pass. `pending_hints` is
/// filled when its block group closes and moved onto the item at the end.
struct TextRecord {
    item_index: usize,
    engine: Range<usize>,
    /// The number of the passage the text is in.
    passage: usize,
    text: String,
    pending_hints: Vec<ResolvedHint<Text>>,
}

/// An item met inside an open passage, pushed once the passage ends.
enum Deferred {
    /// An attribute value.
    Attribute(AttributeSpan),
    /// A comment body.
    Comment(Range<usize>),
}

/// An open element on the parse stack.
struct Frame {
    name: String,
    /// The element name as context words (`paymentCard` → `payment Card`),
    /// pre-computed from the original-case local name at open time; the hint
    /// text attached to the element's content.
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

/// The byte length of a leading UTF-8 BOM (`U+FEFF`), or 0 if absent. quick-xml
/// skips it and reports positions past it, so spans over the original bytes must
/// add it back.
pub(crate) fn bom_len(raw: &str) -> usize {
    if raw.starts_with('\u{feff}') { 3 } else { 0 }
}

/// The lowercased name of the start tag `e`, read from `raw`: as a browser
/// reads it in `html`, its local name in XML. HTML is case-insensitive, so
/// lowercasing makes `<SCRIPT>` and `<P>` match too.
pub(crate) fn start_name(e: &BytesStart<'_>, raw: &str, html: bool) -> Result<String> {
    if html {
        return Ok(e.html5_tag(raw)?.name.to_ascii_lowercase());
    }
    Ok(e.local_name().as_ref().to_ascii_lowercase())
}

/// The end tag's lowercased name, read as [`start_name`] reads a start tag.
pub(crate) fn end_name(e: &BytesEnd<'_>, raw: &str, html: bool) -> Result<String> {
    if html {
        return Ok(e.html5_tag(raw)?.name.to_ascii_lowercase());
    }
    Ok(e.local_name().as_ref().to_ascii_lowercase())
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

/// Give each text record in a block group one hint per record of the group in
/// another passage, located at that sibling's engine-space span: the context a
/// boost points back at when a label and its value sit in separate passages of
/// one block (`<td>Card number<p>4111…</p></td>`). A sibling in the record's own
/// passage is already in its chunk, so it adds no hint.
fn attach_sibling_hints(group: &mut [TextRecord]) {
    let siblings: Vec<(Range<usize>, usize, String)> = group
        .iter()
        .filter(|r| !r.text.is_empty())
        .map(|r| (r.engine.clone(), r.passage, r.text.clone()))
        .collect();
    if siblings.len() < 2 || siblings.len() > MAX_SIBLING_HINT_GROUP {
        return;
    }
    for record in group.iter_mut() {
        let hints = siblings
            .iter()
            .filter(|(_, passage, _)| *passage != record.passage)
            .map(|(engine, _, text)| {
                ResolvedHint::new(
                    TextLocation::inline(engine.clone()),
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
    use std::sync::Arc;

    use elide_codec::Stream;
    use elide_codec::extract::{ExtractStream, SharedSplice};
    use elide_core::modality::Chunk;

    use super::*;
    use crate::engine::address::{MarkupAddresser, XmlSpan};
    use crate::xml::FORMAT_ID;

    /// Build a body stream over `raw` with an explicit config so a test can read
    /// out what the parser extracted (values and hints).
    fn handler_with(raw: &str, config: MarkupConfig<'_>) -> ExtractStream<XmlSpan> {
        let items = build_items(raw, config).expect("markup decode succeeds");
        let state = SharedSplice::new(items);
        ExtractStream::new(FORMAT_ID, state, Arc::new(MarkupAddresser))
    }

    fn xml(raw: &str) -> ExtractStream<XmlSpan> {
        handler_with(raw, MarkupConfig::xml())
    }

    fn values(h: &ExtractStream<XmlSpan>) -> Vec<String> {
        h.chunks()
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.data.as_str().to_owned())
            .collect()
    }

    /// The first chunk a stream yields.
    fn first_chunk(h: &ExtractStream<XmlSpan>) -> Chunk<Text> {
        h.chunks().unwrap().into_iter().next().expect("one chunk")
    }

    #[tokio::test]
    async fn the_element_name_hints_its_text() {
        // `<paymentCard>` should reach its text as a `payment Card` hint, so a
        // context keyword (`card`) can boost the value inside, the markup
        // counterpart of a JSON key or CSV header vouching for its value.
        let raw = "<paymentCard>4111 1111 1111 1111</paymentCard>";
        let h = xml(raw);
        let chunk = first_chunk(&h);
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
    async fn an_attribute_value_is_hinted_by_its_name() {
        // `ssn="123-45-6789"` reaches its value as an `ssn` hint located at the
        // attribute name, so a context keyword can boost the value. This also
        // pins that the name's source span resolves (the hint text and location
        // both come from slicing the source at the name).
        let raw = r#"<r paymentCard="4111 1111 1111 1111"/>"#;
        let h = xml(raw);
        let chunk = first_chunk(&h);
        assert_eq!(chunk.data.as_str(), "4111 1111 1111 1111");
        let hint = chunk.hints.first().expect("attribute-name hint present");
        // The name is split into context words for word-boundary matching.
        assert_eq!(hint.data.as_str(), "payment Card");
        // A raw-source location (not a decoded range), pointing at the attribute
        // name in the source: a consumer resolves it against the source bytes.
        assert!(
            hint.hint.location.range().is_none(),
            "source-only, no decoded range"
        );
        let refs = hint.hint.location.source_refs();
        assert_eq!(refs.len(), 1);
        assert_eq!(&raw[refs[0].range.clone()], "paymentCard");
    }

    #[tokio::test]
    async fn a_namespaced_attribute_hint_keys_on_the_local_name() {
        // A namespace prefix (`c:ssn`) is stripped so the hint keys on the local
        // name, matching how the element path uses the local name.
        let raw = r#"<r c:ssn="123-45-6789"/>"#;
        let h = xml(raw);
        let chunk = first_chunk(&h);
        let hint = chunk.hints.first().expect("attribute-name hint present");
        assert_eq!(hint.data.as_str(), "ssn");
    }

    #[tokio::test]
    async fn a_leading_bom_does_not_shift_extracted_text() {
        // quick-xml reports positions past a leading BOM; the extractor must add
        // it back so the streamed text is exact (not shifted into garbage) and a
        // recognizer can match it.
        let raw = "\u{feff}<?xml version=\"1.0\"?><r>alice@example.com</r>";
        let h = xml(raw);
        let vs = values(&h);
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
        let h = xml(raw);
        let vs = values(&h);
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
            let h = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &["script"]));
            let vs = values(&h);
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
    #[test]
    fn html_text_reads_on_across_inline_elements_until_a_block() {
        let raw = concat!(
            "<div><p>Contact <b>jane.roe@</b>example.com <a href=\"tel:x\">now</a><br>Rack 7</p>",
            "<p>Next</p></div>",
        );
        let h = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &[]));
        assert_eq!(
            values(&h),
            ["Contact jane.roe@example.com now\nRack 7", "tel:x", "Next"],
            "the passage is unbroken, and the link's attribute follows it",
        );
    }

    #[test]
    fn xml_reads_each_text_node_as_its_own_passage() {
        let h = xml("<name><first>Jane</first><last>Roe</last></name>");
        assert_eq!(values(&h), ["Jane", "Roe"]);
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
        let h = handler_with(raw, MarkupConfig::lenient(TEST_BLOCKS, &[]));
        let mut hinted = false;
        for chunk in h.chunks().unwrap() {
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
