//! [`MarkupDocument`]: a decoded markup document, its body stream and how it is
//! re-serialised, and [`markup_document`], which assembles one.
//!
//! Re-serialisation preserves the document **verbatim**: [`MarkupDocument`]
//! writes back only the stretch of each item's value a redaction changed,
//! escaped for where it lands, leaving the declaration, whitespace, attribute
//! quoting, character references, and everything outside the redacted
//! stretches byte-identical. Splices apply right-to-left so an earlier edit's
//! length delta never shifts a later span.

use std::cmp::Reverse;
use std::ops::Range;

use elide_codec::content::ContentData;
use elide_codec::extract::{ExtractStream, Passage};
use elide_codec::{Blob, Document, FormatId, LocalId, Native, StreamMut};
use elide_core::modality::text::Text;
use elide_core::{Error, Result};

use super::address::{MarkupAddresser, XmlItem, XmlSpan};
use crate::sanitize::MarkupSanitizer;

/// The id of a markup document's one stream.
const BODY_ID: &str = "body";

/// A decoded markup document: its body [`ExtractStream`] over the retained raw
/// source, re-serialised by splicing each item back at its source span, then
/// checked against the sanitizer the decode applied.
pub(crate) struct MarkupDocument {
    /// The body items.
    body: ExtractStream<XmlSpan>,
    /// The retained (sanitized) raw document.
    raw: String,
    /// The sanitizer the decode applied, which the output must pass.
    sanitizer: MarkupSanitizer,
}

impl Native for MarkupDocument {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        vec![(
            LocalId::new(BODY_ID),
            StreamMut::new::<Text, _>(&mut self.body),
        )]
    }

    fn encode(&self, _blobs: &[Blob], _removed: &[LocalId]) -> Result<ContentData> {
        let out = splice(&self.raw, self.body.items())?;
        Ok(ContentData::new(out.into_bytes().into()))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        self.sanitizer.check(&output.decode()?)
    }
}

/// Assemble a markup [`Document`] under `format_id` from the parsed `items` over
/// the retained (sanitized) `raw` source, checked on encode against
/// `sanitizer`. Shared by the XML and HTML loaders.
pub(crate) fn markup_document(
    format_id: FormatId,
    raw: String,
    items: Vec<XmlItem>,
    sanitizer: MarkupSanitizer,
) -> Document {
    let native = MarkupDocument {
        body: ExtractStream::new(format_id, items, MarkupAddresser),
        raw,
        sanitizer,
    };
    Document::new(format_id, native, Vec::new())
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
    use elide_codec::Stream;
    use elide_core::ErrorKind;
    use elide_core::modality::DataWriter;
    use elide_core::modality::text::{SourceRef, TextLocation, TextReplacement};
    use elide_core::redaction::Redactions;

    use super::*;
    use crate::engine::config::MarkupConfig;
    use crate::engine::parser::build_items;
    use crate::xml::FORMAT_ID;

    /// A markup body stream over its raw source, spliced back the way
    /// [`MarkupDocument`] does.
    struct Doc {
        stream: ExtractStream<XmlSpan>,
        raw: String,
    }

    impl Doc {
        fn new(raw: &str, config: MarkupConfig<'_>) -> Self {
            let items = build_items(raw, config).expect("markup decode succeeds");
            Doc {
                stream: ExtractStream::new(FORMAT_ID, items, MarkupAddresser),
                raw: raw.to_owned(),
            }
        }

        fn encoded(&self) -> String {
            splice(&self.raw, self.stream.items()).unwrap()
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
        doc.stream
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("[NAME]"),
            )]))
            .await
            .unwrap();
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
        let err = doc
            .stream
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("[NAME]"),
            )]))
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
        let err = doc
            .stream
            .write_at(Redactions::from([(
                location,
                TextReplacement::substituted("[NAME]"),
            )]))
            .await
            .expect_err("out-of-range source must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    #[tokio::test]
    async fn redact_text_node() {
        let raw = "<root><name>Alice</name></root>";
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "Alice");
        doc.stream
            .write_at(Redactions::from([(
                chunk.location,
                TextReplacement::substituted("[NAME]"),
            )]))
            .await
            .unwrap();
        assert_eq!(doc.encoded(), "<root><name>[NAME]</name></root>");
    }

    #[tokio::test]
    async fn redact_attribute_value() {
        let raw = r#"<user email="alice@example.com">Bob</user>"#;
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "alice@example.com");
        doc.stream
            .write_at(Redactions::from([(
                chunk.location,
                TextReplacement::substituted("[EMAIL]"),
            )]))
            .await
            .unwrap();
        assert_eq!(doc.encoded(), r#"<user email="[EMAIL]">Bob</user>"#);
    }

    #[tokio::test]
    async fn redact_cdata_body() {
        let raw = "<doc><![CDATA[alice@example.com]]></doc>";
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "alice@example.com");
        doc.stream
            .write_at(Redactions::from([(
                chunk.location,
                TextReplacement::substituted("[EMAIL]"),
            )]))
            .await
            .unwrap();
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
        doc.stream
            .write_at(Redactions::from([(
                loc,
                TextReplacement::substituted("[EMAIL]"),
            )]))
            .await
            .unwrap();
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
        doc.stream
            .write_at(Redactions::from([(
                loc,
                TextReplacement::substituted("[EMAIL]"),
            )]))
            .await
            .unwrap();
        assert_eq!(
            doc.encoded(),
            r#"<p><script>var a="keep@x.com";</script>mail [EMAIL]</p>"#
        );
    }
}
