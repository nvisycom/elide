//! XML codec side: the [`XmlSpan`] address, the [`Format`] descriptor, and the
//! [`MarkupAddresser`] / [`MarkupRecombine`] the shared markup engine (HTML runs
//! on it too) uses to map and re-serialise a mutated item stream.
//!
//! Re-serialisation preserves the document **verbatim**: [`MarkupRecombine`]
//! writes back only the stretch of each item's value a redaction changed,
//! escaped for where it lands, leaving the declaration, whitespace, attribute
//! quoting, character references, and everything outside the redacted
//! stretches byte-identical. Splices apply right-to-left so an earlier edit's
//! length delta never shifts a later span.

use std::cmp::Reverse;
use std::ops::Range;
use std::sync::Arc;

use elide_codec::content::ContentData;
use elide_codec::extract::{
    CharacterReferences, ExtractStream, ExtractedItem, ItemEdit, OffsetMap, Passage, SharedSplice,
    SourceAddresser, decode_references,
};
use elide_codec::{
    Document, DocumentPart, EncodedPart, ErasedStream, Format, FormatId, LocalId, Recombine, Stream,
};
use elide_core::modality::text::{SourceRef, Text};
use elide_core::{Error, Result};
use quick_xml::escape::{escape, partial_escape};

use super::XmlLoader;
use super::sanitize::MarkupSanitizer;

/// Stable [`FormatId`] for the XML codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.xml");

/// An XML [`ExtractedItem`] addressed by the source bytes its `value` was read
/// from.
///
/// [`ExtractedItem`]: elide_codec::extract::ExtractedItem
pub(crate) type XmlItem = ExtractedItem<XmlSpan>;

/// Where an [`ExtractedItem`]'s value was read from in the retained raw
/// document, and how a new value is written back there.
///
/// The value is the text a reader sees: a text run's or an attribute value's
/// character references (`&amp;`, `&#64;`, HTML's `&nbsp;`) are decoded, and
/// [`offsets`](Self::offsets) maps it back onto the raw bytes.
///
/// [`ExtractedItem`]: elide_codec::extract::ExtractedItem
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XmlSpan {
    /// The *inner* raw bytes the value was read from: a text run, an
    /// attribute value between its quotes, a comment body between `<!--` and
    /// `-->`, a CDATA payload between `<![CDATA[` and `]]>`.
    pub(super) range: Range<usize>,
    /// The map from the value back onto [`range`](Self::range).
    pub(super) offsets: OffsetMap,
    /// How a new value is escaped where it lands.
    pub(super) kind: SpanKind,
    /// The value as read, which a write-back compares the current value
    /// against.
    pub(super) read: String,
}

/// How a new value is escaped where an [`XmlSpan`] lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpanKind {
    /// Character data: `<`, `>`, and `&` are escaped.
    Text,
    /// An attribute value: quotes are escaped too, so the value cannot end
    /// early.
    Attribute,
    /// A comment body or CDATA payload, written as it is.
    Verbatim,
}

impl XmlSpan {
    /// The value of the escaped `kind` text at `range` of `raw`, its
    /// references decoded by `references`, and its span.
    pub(super) fn escaped(
        raw: &str,
        range: Range<usize>,
        kind: SpanKind,
        references: CharacterReferences,
    ) -> (String, Self) {
        let (value, offsets) = decode_references(&raw[range.clone()], range.start, references);
        (value.clone(), Self::new(range, offsets, kind, value))
    }

    /// The value of the verbatim text at `range` of `raw`, and its span.
    pub(super) fn verbatim(raw: &str, range: Range<usize>) -> (String, Self) {
        let value = raw[range.clone()].to_owned();
        let offsets = OffsetMap::identity(range.start, range.len());
        (
            value.clone(),
            Self::new(range, offsets, SpanKind::Verbatim, value),
        )
    }

    /// The span of a separator reading as `value`, which stands for the markup
    /// at `range` and maps to no source text.
    pub(super) fn separator(range: Range<usize>, value: &str) -> (String, Self) {
        let span = Self::new(
            range,
            OffsetMap::default(),
            SpanKind::Verbatim,
            value.to_owned(),
        );
        (value.to_owned(), span)
    }

    fn new(range: Range<usize>, offsets: OffsetMap, kind: SpanKind, read: String) -> Self {
        Self {
            range,
            offsets,
            kind,
            read,
        }
    }

    /// The raw range to replace, and its new raw text, that write `value` back
    /// over this span; `None` while `value` is as read.
    ///
    /// Only the stretch between the longest unchanged start and end is
    /// rewritten, widened to whole character references, so a reference the
    /// redaction did not touch keeps its spelling.
    fn write_back(&self, value: &str) -> Option<(Range<usize>, String)> {
        let read = self.read.as_str();
        if value == read {
            return None;
        }
        let prefix = common_prefix(read, value);
        let suffix = common_suffix(&read[prefix..], &value[prefix..]);
        let mut changed = prefix..read.len() - suffix;
        // A pure insertion covers no read text; widen it by one character so it
        // lands on raw bytes.
        if changed.is_empty() {
            match read[changed.end..].chars().next() {
                Some(next) => changed.end += next.len_utf8(),
                None => changed.start -= read[..changed.start].chars().next_back()?.len_utf8(),
            }
        }
        let raw = self.offsets.raw_ranges(changed);
        let raw = raw.first()?.start..raw.last()?.end;
        let decoded = self.offsets.decoded_ranges(raw.clone());
        let decoded = decoded.first()?.start..decoded.last()?.end;
        let text = &value[decoded.start..value.len() - (read.len() - decoded.end)];
        let text = match self.kind {
            SpanKind::Text => partial_escape(text),
            SpanKind::Attribute => escape(text),
            SpanKind::Verbatim => text.into(),
        };
        Some((raw, text.into_owned()))
    }
}

/// The byte length of the longest common prefix of `a` and `b`, on a character
/// boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((at, _), _)| at)
}

/// The byte length of the longest common suffix of `a` and `b`, on a character
/// boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

/// [`Format`] descriptor registered into `FormatRegistry`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID.clone(), XmlLoader)
        .with_extensions(["xml"])
        .with_content_types(["application/xml", "text/xml"])
}

/// Maps a markup item's decoded value range to/from its raw source bytes
/// through its [`XmlSpan`]'s offset map, single-file (no part tag).
#[derive(Debug, Default)]
pub(crate) struct MarkupAddresser;

impl SourceAddresser<XmlSpan> for MarkupAddresser {
    fn source_span(&self, item: &XmlItem, local: Range<usize>) -> Vec<SourceRef> {
        item.address
            .offsets
            .raw_ranges(local)
            .into_iter()
            .map(SourceRef::new)
            .collect()
    }

    fn locate_source(&self, items: &[XmlItem], source: &[SourceRef]) -> Vec<ItemEdit> {
        // XML is a single file, so its source references carry no part; reject
        // any part-tagged reference rather than misresolve it. Each reference
        // must lie within one item.
        let mut edits = Vec::with_capacity(source.len());
        for reference in source {
            let raw = &reference.range;
            let item = items.iter().enumerate().find(|(_, item)| {
                item.passage != Passage::Separator
                    && item.address.range.start <= raw.start
                    && raw.end <= item.address.range.end
            });
            let (None, Some((item, found))) = (&reference.part, item) else {
                return Vec::new();
            };
            let decoded = found.address.offsets.decoded_ranges(raw.clone());
            let (Some(first), Some(last)) = (decoded.first(), decoded.last()) else {
                return Vec::new();
            };
            edits.push(ItemEdit {
                item,
                local: first.start..last.end,
            });
        }
        ItemEdit::merged(edits)
    }
}

/// Re-serialises a markup document by splicing each item's current value back at
/// its source span into the retained raw string. Holds the shared item state the
/// [`ExtractStream`] redacts in place, and
/// the sanitizer the output is checked against.
#[derive(Debug)]
pub(crate) struct MarkupRecombine {
    /// The retained (sanitized) raw document.
    pub(super) raw: String,
    /// The (redacted-in-place) item stream, shared with the body stream.
    pub(super) state: SharedSplice<XmlSpan>,
    /// The sanitizer the decode applied, which the output must pass.
    pub(super) sanitizer: MarkupSanitizer,
}

impl Recombine for MarkupRecombine {
    fn assemble(&self, _parts: &[EncodedPart], _removed: &[LocalId]) -> Result<ContentData> {
        // The body stream's `EncodedPart` bytes are the ignored marker (a spliced
        // body has no standalone bytes); the redacted items live in the shared
        // state, spliced back over the retained raw source.
        let out = self.state.with_items(|items| splice(&self.raw, items))?;
        Ok(ContentData::new(out.into_bytes().into()))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        self.sanitizer.check(&output.decode()?)
    }
}

/// Assemble a leaf markup [`Document`] under `format_id` from the parsed
/// `items` over the retained (sanitized) `raw` source: one body
/// [`ExtractStream`] sharing its item state with a [`MarkupRecombine`] that
/// checks its output against `sanitizer`. Shared by the XML and HTML loaders.
pub(crate) fn markup_document(
    format_id: FormatId,
    raw: String,
    items: Vec<XmlItem>,
    sanitizer: MarkupSanitizer,
) -> Document {
    let state: SharedSplice<XmlSpan> = SharedSplice::new(items);
    let stream = ExtractStream::new(format_id.clone(), state.clone(), Arc::new(MarkupAddresser));
    Document::new(
        format_id.clone(),
        vec![DocumentPart::Stream {
            id: LocalId::new("body"),
            handle: ErasedStream::new(format_id, Box::new(stream) as Box<dyn Stream<Text>>),
        }],
        Box::new(MarkupRecombine {
            raw,
            state,
            sanitizer,
        }),
    )
}

/// Write each item's current value back into `raw`, returning the rebuilt
/// string: only the stretch of each item a redaction changed, escaped for
/// where it lands.
///
/// Item spans come from disjoint quick-xml events over this same `raw`, so
/// they never overlap. Applying them right-to-left means each splice's
/// length delta can't shift the spans of items earlier in the document. A
/// separator stands for its markup and is never written back.
pub(crate) fn splice(raw: &str, items: &[XmlItem]) -> Result<String> {
    let mut edits: Vec<(Range<usize>, String)> = items
        .iter()
        .filter(|item| item.passage != Passage::Separator)
        .filter_map(|item| item.address.write_back(&item.value))
        .collect();
    edits.sort_by_key(|(range, _)| Reverse(range.start));

    let mut out = raw.to_owned();
    for (Range { start, end }, text) in edits {
        // Spans index into `out`, which starts as `raw` and only ever
        // grows/shrinks to the right of the current splice, so they stay
        // in-bounds and on char boundaries by construction. The guards are
        // defensive: a malformed loader would surface here rather than
        // panic in `replace_range`.
        if end > out.len() || start > end {
            return Err(Error::processing(format!(
                "xml splice span {start}..{end} out of bounds (len {})",
                out.len()
            )));
        }
        if !out.is_char_boundary(start) || !out.is_char_boundary(end) {
            return Err(Error::processing(format!(
                "xml splice span {start}..{end} falls mid-character"
            )));
        }
        out.replace_range(start..end, &text);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::modality::DataWriter;
    use elide_core::modality::text::{TextLocation, TextReplacement};
    use elide_core::redaction::{HiddenContentPolicy, Redactions};

    use super::*;
    use crate::markup::config::MarkupConfig;
    use crate::markup::markup_parser::build_items;
    use crate::markup::sanitize::Dialect;

    /// A markup body stream paired with the recombiner that re-serialises it,
    /// both over one shared item state (so a redaction on the stream is visible
    /// on `encode`). Mirrors the `(ExtractStream, MarkupRecombine)` split the
    /// loader assembles into a `Document`.
    struct Doc {
        stream: ExtractStream<XmlSpan>,
        recombine: MarkupRecombine,
    }

    impl Doc {
        fn new(raw: &str, config: MarkupConfig<'_>) -> Self {
            let items = build_items(raw, config).expect("markup decode succeeds");
            let state: SharedSplice<XmlSpan> = SharedSplice::new(items);
            let stream =
                ExtractStream::new(FORMAT_ID.clone(), state.clone(), Arc::new(MarkupAddresser));
            Doc {
                stream,
                recombine: MarkupRecombine {
                    raw: raw.to_owned(),
                    state,
                    sanitizer: MarkupSanitizer::new(Dialect::Xml, HiddenContentPolicy::default()),
                },
            }
        }

        fn encoded(&self) -> String {
            self.recombine.assemble(&[], &[]).unwrap().decode().unwrap()
        }
    }

    fn load(raw: &str) -> Doc {
        Doc::new(raw, MarkupConfig::xml())
    }

    /// The first body chunk whose text satisfies `pred`.
    fn chunk_where(
        doc: &mut Doc,
        pred: impl Fn(&str) -> bool,
    ) -> elide_core::modality::Chunk<Text> {
        doc.stream
            .chunks()
            .unwrap()
            .into_iter()
            .find(|c| pred(c.data.as_str()))
            .expect("chunk")
    }

    /// Redact each of `needles` where it reads in the decoded body, the way the
    /// pipeline does: found in a chunk, lifted to source coordinates, written.
    async fn redact(doc: &mut Doc, needles: &[(&str, &str)]) {
        let mut redactions = Redactions::new();
        for (needle, replacement) in needles {
            let chunk = chunk_where(doc, |text| text.contains(needle));
            let at = chunk.data.as_str().find(needle).unwrap();
            let location = doc
                .stream
                .lift(&chunk, TextLocation::inline(at..at + needle.len()))
                .expect("lifts");
            redactions.push(location, TextReplacement::substituted(*replacement));
        }
        doc.stream.write_at(redactions).await.unwrap();
    }

    #[tokio::test]
    async fn character_references_read_decoded_and_untouched_ones_keep_their_spelling() {
        let raw = "<r><n>Write to jane&#46;roe&#64;example.com &amp; A&#x26;B</n></r>";
        let mut doc = load(raw);
        // The run reads decoded, as one chunk.
        chunk_where(&mut doc, |text| {
            text == "Write to jane.roe@example.com & A&B"
        });
        redact(&mut doc, &[("jane.roe@example.com", "[EMAIL]")]).await;
        assert_eq!(
            doc.encoded(),
            "<r><n>Write to [EMAIL] &amp; A&#x26;B</n></r>"
        );
    }

    #[tokio::test]
    async fn an_attribute_value_reads_decoded_and_a_replacement_is_escaped() {
        let raw = r#"<r><a href="mailto:jane&#64;example.com">x</a><n>Tom</n></r>"#;
        let mut doc = load(raw);
        redact(
            &mut doc,
            &[("jane@example.com", "[EMAIL]"), ("Tom", "<\"name\" & co>")],
        )
        .await;
        assert_eq!(
            doc.encoded(),
            r#"<r><a href="mailto:[EMAIL]">x</a><n>&lt;"name" &amp; co&gt;</n></r>"#
        );
    }

    #[test]
    fn a_write_back_rewrites_only_the_changed_stretch_widened_to_whole_references() {
        let raw = "a&amp;b&amp;c";
        let (_, span) =
            XmlSpan::escaped(raw, 0..raw.len(), SpanKind::Text, CharacterReferences::Xml);
        // "a&b&c" → "a&X&c": the middle `b` alone.
        assert_eq!(span.write_back("a&X&c"), Some((6..7, "X".to_owned())));
        // "a&b&c" → "a?b&c": the first reference, whole.
        assert_eq!(span.write_back("a?b&c"), Some((1..6, "?".to_owned())));
        // An insertion lands on the character after it.
        assert_eq!(span.write_back("a&b&cd"), Some((12..13, "cd".to_owned())));
        assert_eq!(span.write_back("a&b&c"), None);
    }

    #[tokio::test]
    async fn encode_unchanged_round_trips_verbatim() {
        let raw = "<?xml version=\"1.0\"?>\n<root attr=\"x\">\n  <name>Alice</name>\n  <!-- note -->\n</root>\n";
        let doc = load(raw);
        assert_eq!(doc.encoded(), raw);
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
            let doc = load(raw);
            assert_eq!(doc.encoded(), raw, "round-trip changed: {raw:?}");
        }
    }

    #[tokio::test]
    async fn lift_carries_the_exact_source_span() {
        // Text preceded by tags: the chunk's stream offset differs from the raw
        // byte offset, so `source` must point at the raw bytes, not the stream.
        let raw = "<root><name>Alice Carter</name></root>";
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "Alice Carter");
        // Redact "Carter", value-local [6, 12).
        let lifted = doc
            .stream
            .lift(&chunk, TextLocation::inline(6..12))
            .expect("in bounds");
        // "Carter" sits at raw bytes 18..24 in the document.
        let want = "<root><name>Alice Carter".find("Carter").unwrap();
        assert_eq!(
            lifted.source_refs(),
            &[SourceRef::new(want..want + "Carter".len())][..]
        );
        assert_eq!(&raw[want..want + 6], "Carter");
    }

    #[tokio::test]
    async fn redacts_a_node_located_only_by_source() {
        // A caller with only raw coordinates (no decoded range) locates the node
        // by a bare, part-less `SourceRef`, XML is a single file.
        let raw = "<root><name>Alice</name></root>";
        let mut doc = load(raw);
        let at = raw.find("Alice").unwrap();
        let location = TextLocation::marked_source(SourceRef::new(at..at + "Alice".len()));
        let mut rs = Redactions::new();
        rs.push(location, TextReplacement::substituted("[NAME]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(doc.encoded(), "<root><name>[NAME]</name></root>");
    }

    #[tokio::test]
    async fn a_part_tagged_source_ref_is_rejected_for_xml() {
        // XML is single-file; a part-tagged reference does not address it. The
        // caller supplied an explicit source coordinate to redact, so an
        // unresolvable one is an error, not a silent no-op that would leave a
        // green audit over an unredacted document.
        let raw = "<root><name>Alice</name></root>";
        let mut doc = load(raw);
        let at = raw.find("Alice").unwrap();
        let location = TextLocation::marked_source(SourceRef::in_part(
            at..at + "Alice".len(),
            "some/part.xml",
        ));
        let mut rs = Redactions::new();
        rs.push(location, TextReplacement::substituted("[NAME]"));
        let err = doc
            .stream
            .write_at(rs)
            .await
            .expect_err("part-tagged ref must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    #[tokio::test]
    async fn an_out_of_range_source_ref_is_rejected_for_xml() {
        // A source span past the document resolves to no item, a caller mistake
        // (e.g. an off-by-one in a run→byte mapping), so it errors rather than
        // leaving a green audit over an unredacted document.
        let raw = "<root><name>Alice</name></root>";
        let mut doc = load(raw);
        let past = raw.len() + 4;
        let location = TextLocation::marked_source(SourceRef::new(past..past + 3));
        let mut rs = Redactions::new();
        rs.push(location, TextReplacement::substituted("[NAME]"));
        let err = doc
            .stream
            .write_at(rs)
            .await
            .expect_err("out-of-range source must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    #[tokio::test]
    async fn redact_text_node() {
        let raw = "<root><name>Alice</name></root>";
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "Alice");
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[NAME]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(doc.encoded(), "<root><name>[NAME]</name></root>");
    }

    #[tokio::test]
    async fn redact_attribute_value() {
        let raw = r#"<user email="alice@example.com">Bob</user>"#;
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "alice@example.com");
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[EMAIL]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(doc.encoded(), r#"<user email="[EMAIL]">Bob</user>"#);
    }

    #[tokio::test]
    async fn redact_cdata_body() {
        let raw = "<doc><![CDATA[alice@example.com]]></doc>";
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "alice@example.com");
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[EMAIL]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(doc.encoded(), "<doc><![CDATA[[EMAIL]]]></doc>");
    }

    #[tokio::test]
    async fn redact_partial_text() {
        let raw = "<p>contact alice@example.com today</p>";
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t.contains("alice@example.com"));
        let at = chunk.data.as_str().find("alice@example.com").unwrap();
        let loc = TextLocation::inline(
            chunk.location.range().unwrap().start + at
                ..chunk.location.range().unwrap().start + at + "alice@example.com".len(),
        );
        let mut rs = Redactions::new();
        rs.push(loc, TextReplacement::substituted("[EMAIL]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(doc.encoded(), "<p>contact [EMAIL] today</p>");
    }

    // A lenient round-trip over the same engine, exercising our splice
    // bookkeeping across a skipped region. Gated on `html`: the lenient config
    // only exists when HTML is compiled. A small synthetic vocabulary stands in
    // for the real HTML element lists, which the HTML loader tests directly.
    #[cfg(feature = "html")]
    const TEST_BLOCKS: &[&str] = &["p", "div"];

    #[cfg(feature = "html")]
    #[tokio::test]
    async fn redaction_after_a_skipped_script_is_byte_faithful() {
        // Redacting text that follows a skipped script leaves the script and all
        // other markup byte-identical, changing only the targeted span, the
        // splice offsets stay correct across the skipped region.
        let raw = r#"<p><script>var a="keep@x.com";</script>mail alice@example.com</p>"#;
        let mut doc = Doc::new(raw, MarkupConfig::lenient(TEST_BLOCKS, &["script"]));
        let chunk = chunk_where(&mut doc, |t| t.contains("alice@example.com"));
        let at = chunk.data.as_str().find("alice@example.com").unwrap();
        let loc = TextLocation::inline(
            chunk.location.range().unwrap().start + at
                ..chunk.location.range().unwrap().start + at + "alice@example.com".len(),
        );
        let mut rs = Redactions::new();
        rs.push(loc, TextReplacement::substituted("[EMAIL]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(
            doc.encoded(),
            r#"<p><script>var a="keep@x.com";</script>mail [EMAIL]</p>"#
        );
    }
}
