//! XML codec side: the [`XmlSpan`] address, the [`Format`] descriptor, and the
//! [`MarkupAddresser`] / [`MarkupRecombine`] the shared markup engine (HTML runs
//! on it too) uses to map and re-serialise a mutated item stream.
//!
//! Re-serialisation preserves the document **verbatim**: [`MarkupRecombine`]
//! splices each item's current value back at its recorded source byte span into
//! the retained raw string, leaving the declaration, whitespace, attribute
//! quoting, and everything outside the redacted spans byte-identical. Splices
//! apply right-to-left so an earlier edit's length delta never shifts a later
//! span.

use std::cmp::Reverse;
use std::ops::Range;
use std::sync::Arc;

use elide_codec::content::ContentData;
use elide_codec::extract::{ExtractStream, ExtractedItem, ItemEdit, SharedSplice, SourceAddresser};
use elide_codec::{
    Document, DocumentPart, EncodedPart, ErasedStream, Format, FormatId, LocalId, Recombine, Stream,
};
use elide_core::modality::text::{SourceRef, Text};
use elide_core::{Error, ErrorKind, Result};

use super::XmlLoader;

/// Stable [`FormatId`] for the XML codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.xml");

/// An XML [`ExtractedItem`] addressed by the source byte span its
/// `value` occupies in the original document.
///
/// [`ExtractedItem`]: elide_codec::extract::ExtractedItem
pub(crate) type XmlItem = ExtractedItem<XmlSpan>;

/// The source byte span (in the retained raw document) that an
/// [`ExtractedItem`]'s value occupies: the region the recombiner
/// overwrites. These are the *inner* bytes: a text node's text, an
/// attribute value between the quotes, a comment body between `<!--` and
/// `-->`, a CDATA payload between `<![CDATA[` and `]]>`.
///
/// [`ExtractedItem`]: elide_codec::extract::ExtractedItem
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XmlSpan(pub(super) Range<usize>);

/// [`Format`] descriptor registered into `FormatRegistry`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID.clone(), XmlLoader)
        .with_extensions(["xml"])
        .with_content_types(["application/xml", "text/xml"])
}

/// Maps a markup item's decoded value range to/from its raw source span. The
/// item value is the verbatim source slice at its [`XmlSpan`], so the mapping is
/// a bounded offset add (forward) / subtract (reverse); at most one range (the
/// value carries no entity to split on), single-file (no part tag).
#[derive(Debug, Default)]
pub(crate) struct MarkupAddresser;

impl SourceAddresser<XmlSpan> for MarkupAddresser {
    fn source_span(&self, item: &XmlItem, local: Range<usize>) -> Vec<SourceRef> {
        let base = &item.address.0;
        let mapped = base
            .start
            .checked_add(local.start)
            .zip(base.start.checked_add(local.end))
            .filter(|&(start, end)| start <= end && end <= base.end)
            .map(|(start, end)| SourceRef::new(start..end));
        mapped.into_iter().collect()
    }

    fn locate_source(&self, items: &[XmlItem], source: &[SourceRef]) -> Option<ItemEdit> {
        // XML is a single file, so its source references carry no part; reject
        // any part-tagged reference rather than misresolve it.
        if source.iter().any(|s| s.part.is_some()) {
            return None;
        }
        let raw_start = source.iter().map(|s| s.range.start).min()?;
        let raw_end = source.iter().map(|s| s.range.end).max()?;
        let (item, base) = items
            .iter()
            .map(|item| &item.address.0)
            .enumerate()
            .find(|(_, base)| base.start <= raw_start && raw_end <= base.end)?;
        Some(ItemEdit {
            item,
            local: (raw_start - base.start)..(raw_end - base.start),
        })
    }
}

/// Re-serialises a markup document by splicing each item's current value back at
/// its source span into the retained raw string. Holds the shared item state the
/// [`ExtractStream`](elide_codec::extract::ExtractStream) redacts in place.
#[derive(Debug)]
pub(crate) struct MarkupRecombine {
    /// The retained raw document.
    pub(super) raw: String,
    /// The (redacted-in-place) item stream, shared with the body stream.
    pub(super) state: SharedSplice<XmlSpan>,
}

impl Recombine for MarkupRecombine {
    fn assemble(&self, _parts: &[EncodedPart]) -> Result<ContentData> {
        // The body stream's `EncodedPart` bytes are the ignored marker (a spliced
        // body has no standalone bytes); the redacted items live in the shared
        // state, spliced back over the retained raw source.
        let out = self.state.with_items(|items| splice(&self.raw, items))?;
        Ok(ContentData::new(out.into_bytes().into()))
    }
}

/// Assemble a leaf markup [`Document`] under `format_id` from the parsed
/// `items` over the retained `raw` source: one body [`ExtractStream`] sharing
/// its item state with a [`MarkupRecombine`]. Shared by the XML and HTML loaders.
pub(crate) fn markup_document(format_id: FormatId, raw: String, items: Vec<XmlItem>) -> Document {
    let state: SharedSplice<XmlSpan> = SharedSplice::new(items);
    let stream = ExtractStream::new(format_id.clone(), state.clone(), Arc::new(MarkupAddresser));
    Document::new(
        format_id.clone(),
        vec![DocumentPart::Stream {
            id: LocalId::new("body"),
            handle: ErasedStream::new(format_id, Box::new(stream) as Box<dyn Stream<Text>>),
        }],
        Box::new(MarkupRecombine { raw, state }),
    )
}

/// Splice each item's current value back at its source span into `raw`,
/// returning the rebuilt string. Shared by the markup recombiner and by
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
    use elide_core::redaction::Redactions;

    use super::*;
    use crate::markup::config::MarkupConfig;
    use crate::markup::markup_parser::build_items;

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
                },
            }
        }

        fn encoded(&self) -> String {
            self.recombine.assemble(&[]).unwrap().decode().unwrap()
        }
    }

    fn load(raw: &str) -> Doc {
        Doc::new(raw, MarkupConfig::xml())
    }

    /// Read chunks from the body until one whose text satisfies `pred`.
    async fn chunk_where(
        doc: &mut Doc,
        pred: impl Fn(&str) -> bool,
    ) -> elide_core::modality::Chunk<Text> {
        loop {
            let c = doc.stream.read_next().await.unwrap().unwrap();
            if pred(c.data.as_str()) {
                return c;
            }
        }
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
        let chunk = chunk_where(&mut doc, |t| t == "Alice Carter").await;
        // Redact "Carter", value-local [6, 12).
        let lifted = doc
            .stream
            .lift(&chunk, TextLocation::new(6, 12))
            .expect("in bounds");
        // "Carter" sits at raw bytes 18..24 in the document.
        let want = "<root><name>Alice Carter".find("Carter").unwrap();
        assert_eq!(
            lifted.source(),
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
        let location = TextLocation::from_source([SourceRef::new(at..at + "Alice".len())]);
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
        let location = TextLocation::from_source([SourceRef::in_part(
            at..at + "Alice".len(),
            "some/part.xml",
        )]);
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
        let location = TextLocation::from_source([SourceRef::new(past..past + 3)]);
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
        let chunk = chunk_where(&mut doc, |t| t == "Alice").await;
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[NAME]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(doc.encoded(), "<root><name>[NAME]</name></root>");
    }

    #[tokio::test]
    async fn redact_attribute_value() {
        let raw = r#"<user email="alice@example.com">Bob</user>"#;
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "alice@example.com").await;
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[EMAIL]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(doc.encoded(), r#"<user email="[EMAIL]">Bob</user>"#);
    }

    #[tokio::test]
    async fn redact_cdata_body() {
        let raw = "<doc><![CDATA[alice@example.com]]></doc>";
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t == "alice@example.com").await;
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[EMAIL]"));
        doc.stream.write_at(rs).await.unwrap();
        assert_eq!(doc.encoded(), "<doc><![CDATA[[EMAIL]]]></doc>");
    }

    #[tokio::test]
    async fn redact_partial_text() {
        let raw = "<p>contact alice@example.com today</p>";
        let mut doc = load(raw);
        let chunk = chunk_where(&mut doc, |t| t.contains("alice@example.com")).await;
        let at = chunk.data.as_str().find("alice@example.com").unwrap();
        let loc = TextLocation::new(
            chunk.location.range().unwrap().start + at,
            chunk.location.range().unwrap().start + at + "alice@example.com".len(),
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
        let chunk = chunk_where(&mut doc, |t| t.contains("alice@example.com")).await;
        let at = chunk.data.as_str().find("alice@example.com").unwrap();
        let loc = TextLocation::new(
            chunk.location.range().unwrap().start + at,
            chunk.location.range().unwrap().start + at + "alice@example.com".len(),
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
