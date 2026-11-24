//! [`ExtractStream<A>`]: the [`Stream`] machinery over an extracted item
//! stream.

use std::ops::Range;

use elide_core::modality::text::{Marked, SourceRef, Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use super::{ExtractedItem, ItemEdit, Passage, SourceAddresser, SpliceState};
use crate::string::DisjointSpans;
use crate::{FlaggedItem, FormatId, Stream};

/// The [`Stream`] over a structured document's extracted, redactable items.
///
/// It yields each [`Passage`] of items as one [`Chunk`], so an entity a
/// document stores across several items is read whole; reads and redacts by
/// decoded offset, across items; and lifts a chunk-local finding to source
/// coordinates via the [`SourceAddresser`]. It owns the decoded items; the
/// format's [`Native`](crate::Native) document reads the redacted ones through
/// [`items`](Self::items) to write the document out.
///
/// [`Chunk`]: elide_core::modality::Chunk
pub struct ExtractStream<A> {
    format_id: FormatId,
    state: SpliceState<A>,
    addresser: Box<dyn SourceAddresser<A>>,
}

impl<A: Send + Sync + 'static> ExtractStream<A> {
    /// Build a stream over the decoded `items`, addressed by `addresser`.
    pub fn new(
        format_id: FormatId,
        items: Vec<ExtractedItem<A>>,
        addresser: impl SourceAddresser<A> + 'static,
    ) -> Self {
        Self {
            format_id,
            state: SpliceState::new(items),
            addresser: Box::new(addresser),
        }
    }

    /// The (possibly redacted) items, in document order.
    pub fn items(&self) -> &[ExtractedItem<A>] {
        &self.state.items
    }

    /// The location of the decoded range `range`, with the exact raw source
    /// range(s) it came from, item by item, when the addresser can map them
    /// (markup, whose item value is a verbatim source slice; OOXML, via its
    /// per-block offset map). A separator has no source text of its own.
    ///
    /// The addresser maps an item's value as read, so each part of `range` is
    /// first taken back to the value as read: after a redaction, what it wrote
    /// maps onto the source bytes it replaced, and the rest onto its own.
    fn source_location(&self, range: Range<usize>) -> TextLocation {
        let state = &self.state;
        let source: Vec<SourceRef> = state
            .covering(range.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|edit| {
                !edit.local.is_empty() && state.items[edit.item].passage != Passage::Separator
            })
            .filter_map(|edit| {
                let read = state.read_range(edit.item, edit.local)?;
                Some(self.addresser.source_span(&state.items[edit.item], read))
            })
            .flatten()
            .collect();
        TextLocation::source(range, source)
    }

    /// The decoded span `location` addresses, checked against the text.
    ///
    /// A resolved location is addressed by its decoded
    /// [`range`](TextLocation::range) alone, the span a read covers; its
    /// source references describe it but never narrow it. A reviewer mark has
    /// no span yet: each of its references is reverse-resolved via
    /// [`SourceAddresser::locate_source`], and together they must cover one
    /// run of text.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] for a location that resolves to nothing, never a
    /// no-op: a span outside the text or splitting a character, a bad part
    /// name, an offset past the part, a cross-part span, or references with
    /// text between them would leave bytes unredacted under a green audit.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn span_for(&self, location: &TextLocation) -> Result<Range<usize>> {
        let state = &self.state;
        let TextLocation::Marked(mark) = location else {
            let range = location
                .range()
                .cloned()
                .ok_or_else(|| Error::malformed("a location with no span"))?;
            state.text(range.clone())?;
            return Ok(range);
        };
        let Marked::Source { refs } = mark else {
            return Err(Error::malformed(
                "a rendered-page mark does not address a structured document",
            ));
        };
        // The addresser locates a reference in an item's value as read; take
        // each back to the value as it now reads.
        let pieces: Vec<ItemEdit> = self
            .addresser
            .locate_source(&state.items, refs)
            .into_iter()
            .map(|ItemEdit { item, local }| {
                Some(ItemEdit {
                    local: state.current_range(item, local)?,
                    item,
                })
            })
            .collect::<Option<_>>()
            .unwrap_or_default();
        if pieces.is_empty() {
            let refs: Vec<String> = refs
                .iter()
                .map(|src| match &src.part {
                    Some(part) => format!("{part}#{}..{}", src.range.start, src.range.end),
                    None => format!("{}..{}", src.range.start, src.range.end),
                })
                .collect();
            return Err(Error::malformed(format!(
                "source reference resolves to no redactable item: {}",
                refs.join(" ")
            )));
        }
        state.contiguous_span(&pieces)
    }
}

impl<A: Send + Sync + 'static> Stream<Text> for ExtractStream<A> {
    fn format(&self) -> FormatId {
        self.format_id
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        let state = &self.state;
        Ok(state
            .passages()
            .into_iter()
            .map(|passage| {
                let start = state.item_starts[passage.start];
                let end = state.item_starts[passage.end];
                let items = &state.items[passage];
                Chunk {
                    location: TextLocation::inline(start..end),
                    data: TextData::new(
                        items
                            .iter()
                            .map(|item| item.value.as_str())
                            .collect::<String>(),
                    ),
                    hints: items
                        .iter()
                        .flat_map(|item| item.hints.iter().cloned())
                        .collect(),
                }
            })
            .collect())
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // Items are byte-for-byte the recognizer's view, so lifting is an
        // identity offset add of the chunk-local range against the chunk's
        // start, bounded by its end. Both are decoded coordinates (a chunk yields
        // a decoded range; a recognizer's local finding is a decoded offset).
        let chunk_range = chunk.location.range()?;
        let local_range = local.range()?;
        let base = chunk_range.start;
        let start = base.checked_add(local_range.start)?;
        let end = base.checked_add(local_range.end)?;
        if start > end || end > chunk_range.end {
            return None;
        }
        Some(self.source_location(start..end))
    }

    fn resolve(&self, location: TextLocation) -> Result<TextLocation> {
        let TextLocation::Marked(mark) = &location else {
            return Ok(location);
        };
        let span = self.span_for(&location)?;
        let Marked::Source { refs } = mark else {
            unreachable!("span_for rejects a rendered-page mark");
        };
        Ok(TextLocation::source(span, refs.clone()))
    }

    fn flagged(&self) -> Result<Vec<FlaggedItem<Text>>> {
        let flagged: Vec<_> = {
            let state = &self.state;
            state
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.passage != Passage::Separator)
                .filter_map(|(index, item)| {
                    let flag = item.flag.clone()?;
                    let range = state.item_starts[index]..state.item_starts[index + 1];
                    (!range.is_empty()).then_some((range, flag))
                })
                .collect()
        };
        Ok(flagged
            .into_iter()
            .map(|(range, flag)| {
                FlaggedItem::new(self.source_location(range), flag.label, flag.kind)
            })
            .collect())
    }
}

#[async_trait::async_trait]
impl<A: Send + Sync + 'static> DataReader<Text> for ExtractStream<A> {
    async fn read_at(&self, location: &TextLocation) -> Result<TextData> {
        // Reads address the decoded stream: a mark is resolved to its span
        // before it is read.
        let range = location
            .range()
            .ok_or_else(|| Error::malformed("an unresolved mark has no text to read"))?;
        self.state.text(range.clone()).map(TextData::new)
    }
}

#[async_trait::async_trait]
impl<A: Send + Sync + 'static> DataWriter<Text> for ExtractStream<A> {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        // Resolve every location against the text as it is, before any edit
        // moves it, then apply right to left so an edit's length delta never
        // moves a span still to come.
        let mut edits = Vec::new();
        for (location, replacement) in redactions {
            let span = self.span_for(&location)?;
            edits.push((span, replacement));
        }
        let spans: Vec<Range<usize>> = edits.iter().map(|(span, _)| span.clone()).collect();
        spans.ensure_disjoint()?;
        edits.sort_by_key(|(span, _)| span.start);
        for (span, replacement) in edits.into_iter().rev() {
            self.state.redact(span, &replacement)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::modality::text::TextReplacement;
    use elide_core::modality::{DataReader, ModalityLocation};
    use elide_core::primitive::Color;

    use super::*;
    use crate::extract::ItemEdit;

    /// Addresses nothing: the items carry no source pre-image.
    struct Unaddressed;

    impl SourceAddresser<()> for Unaddressed {}

    /// Addresses each item's value as its own source: a reference is a range
    /// of the concatenated values, within one item. It reads `item.value`, so
    /// it holds only before a write, when each value is as read.
    struct Identity;

    impl SourceAddresser<()> for Identity {
        fn locate_source(
            &self,
            items: &[ExtractedItem<()>],
            source: &[SourceRef],
        ) -> Vec<ItemEdit> {
            let mut starts = vec![0];
            for item in items {
                starts.push(starts.last().unwrap() + item.value.len());
            }
            let mut edits = Vec::new();
            for SourceRef { range, .. } in source {
                let Some(item) = (0..items.len())
                    .find(|&item| starts[item] <= range.start && range.end <= starts[item + 1])
                else {
                    return Vec::new();
                };
                let local = range.start - starts[item]..range.end - starts[item];
                edits.push(ItemEdit { item, local });
            }
            edits
        }
    }

    fn item(value: &str, passage: Passage) -> ExtractedItem<()> {
        ExtractedItem {
            address: (),
            value: value.to_owned(),
            hints: Vec::new(),
            passage,
            flag: None,
        }
    }

    /// Two passages: a name and an email split across items around a line
    /// break, then a passage of its own.
    fn stream() -> ExtractStream<()> {
        let items = vec![
            item("Jane R", Passage::Start),
            item("oe", Passage::Continue),
            item("\n", Passage::Separator),
            item("jane.roe@", Passage::Continue),
            item("example.com", Passage::Continue),
            item("Next", Passage::Start),
        ];
        ExtractStream::new(FormatId::new("test"), items, Unaddressed)
    }

    fn values(stream: &ExtractStream<()>) -> Vec<String> {
        stream
            .items()
            .iter()
            .map(|item| item.value.clone())
            .collect()
    }

    #[test]
    fn each_passage_is_one_chunk() {
        let stream = stream();
        let chunks = stream.chunks().unwrap();
        let texts: Vec<&str> = chunks.iter().map(|chunk| chunk.data.as_str()).collect();
        assert_eq!(texts, ["Jane Roe\njane.roe@example.com", "Next"]);
        assert_eq!(chunks[1].location.range(), Some(&(29..33)));
    }

    #[tokio::test]
    async fn a_redaction_across_items_replaces_in_the_first_and_cuts_the_rest() {
        let mut stream = stream();
        // "Jane Roe\njane.roe@example.com": the whole passage, across the break.
        stream
            .write_at(Redactions::from([(
                TextLocation::inline(0..29),
                TextReplacement::replaced("[X]"),
            )]))
            .await
            .unwrap();
        assert_eq!(values(&stream), ["[X]", "", "\n", "", "", "Next"]);
    }

    #[tokio::test]
    async fn a_redaction_within_a_passage_keeps_the_separator() {
        let mut stream = stream();
        // "Roe": the tail of one item and the whole of the next.
        stream
            .write_at(Redactions::from([
                (
                    TextLocation::inline(5..8),
                    TextReplacement::replaced("[NAME]"),
                ),
                (
                    TextLocation::inline(9..29),
                    TextReplacement::replaced("[EMAIL]"),
                ),
            ]))
            .await
            .unwrap();
        assert_eq!(
            values(&stream),
            ["Jane [NAME]", "", "\n", "[EMAIL]", "", "Next"]
        );
    }

    /// A cover marks each item's part in place, keeping every item's extent
    /// and the separator between them.
    #[tokio::test]
    async fn a_cover_across_items_marks_each_in_place() {
        let mut stream = stream();
        stream
            .write_at(Redactions::from([(
                TextLocation::inline(5..29),
                TextReplacement::covered(Color::BLACK),
            )]))
            .await
            .unwrap();
        assert_eq!(
            values(&stream),
            ["Jane █", "██", "\n", "█████████", "███████████", "Next"]
        );
    }

    #[tokio::test]
    async fn a_read_spans_items() {
        let stream = stream();
        let read = stream.read_at(&TextLocation::inline(5..18)).await.unwrap();
        assert_eq!(read.as_str(), "Roe\njane.roe@");
    }

    fn addressed() -> ExtractStream<()> {
        let mut stream = stream();
        stream.addresser = Box::new(Identity);
        stream
    }

    #[test]
    fn source_refs_resolve_to_the_run_they_cover() {
        let stream = addressed();
        // "Jane R" and "oe": two items, one name.
        let mark =
            TextLocation::marked_sources([SourceRef::new(0..6), SourceRef::new(6..8)]).unwrap();
        assert_eq!(stream.resolve(mark).unwrap().range(), Some(&(0..8)));
        // "oe" and "jane", across the line break between them.
        let mark =
            TextLocation::marked_sources([SourceRef::new(6..8), SourceRef::new(9..13)]).unwrap();
        assert_eq!(stream.resolve(mark).unwrap().range(), Some(&(6..13)));
    }

    #[tokio::test]
    async fn source_refs_with_text_between_them_are_rejected() {
        let mut marked = addressed();
        // "J" and "R" in one item, "ane " between them.
        let within =
            TextLocation::marked_sources([SourceRef::new(0..1), SourceRef::new(5..6)]).unwrap();
        // "Jane" and "jane" in two items, " Roe\n" between them.
        let across =
            TextLocation::marked_sources([SourceRef::new(9..13), SourceRef::new(0..4)]).unwrap();
        for mark in [within, across] {
            let err = marked.resolve(mark.clone()).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::MalformedInput, "{mark:?}");
            let batch = Redactions::from([(mark, TextReplacement::Removed)]);
            assert!(marked.write_at(batch).await.is_err());
        }
        assert_eq!(values(&marked), values(&stream()));
    }

    #[tokio::test]
    async fn overlapping_redactions_are_an_error() {
        let mut marked = addressed();
        // "Jane Roe" by span, and "Roe" by its source refs.
        let batch = Redactions::from([
            (TextLocation::inline(0..8), TextReplacement::Removed),
            (
                TextLocation::marked_sources([SourceRef::new(5..6), SourceRef::new(6..8)]).unwrap(),
                TextReplacement::Removed,
            ),
        ]);
        let err = marked.write_at(batch).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(values(&marked), values(&stream()));
    }

    #[tokio::test]
    async fn a_span_splitting_a_character_changes_nothing() {
        let items = vec![item("é", Passage::Start), item("ABC", Passage::Continue)];
        let mut stream = ExtractStream::new(FormatId::new("test"), items, Unaddressed);
        // Byte 1 is inside "é"; "AB" must survive the failed batch.
        let batch = Redactions::from([(TextLocation::inline(1..4), TextReplacement::Removed)]);
        let err = stream.write_at(batch).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(values(&stream), ["é", "ABC"]);
    }

    #[tokio::test]
    async fn a_cluster_of_a_span_and_a_mark_redacts_the_whole_span() {
        let mut marked = addressed();
        // A detection of "Jane Roe" fused with a mark on "R": the fused
        // location carries only the mark's reference, but its span is the
        // name, which a read covers and the write must too.
        let mark = marked
            .resolve(TextLocation::marked_source(SourceRef::new(5..6)))
            .unwrap();
        let fused = TextLocation::inline(0..8).union(&mark).unwrap();
        assert_eq!(fused.source_refs(), [SourceRef::new(5..6)]);
        let read = marked.read_at(&fused).await.unwrap();
        assert_eq!(read.as_str(), "Jane Roe");
        let batch = Redactions::from([(fused, TextReplacement::Removed)]);
        marked.write_at(batch).await.unwrap();
        assert_eq!(
            values(&marked),
            ["", "", "\n", "jane.roe@", "example.com", "Next"]
        );
    }

    /// A range past the text, or a mark the addresser cannot place, is an
    /// error rather than a skipped entity.
    #[tokio::test]
    async fn an_unaddressable_location_is_an_error() {
        let mut stream = stream();
        let past = TextLocation::inline(0..999);
        assert!(stream.read_at(&past).await.is_err());
        let mark = TextLocation::marked_source(SourceRef::new(0..4));
        assert!(stream.resolve(mark.clone()).is_err());
        assert!(stream.read_at(&mark).await.is_err());
        let batch = Redactions::from([(past, TextReplacement::Removed)]);
        assert!(stream.write_at(batch).await.is_err());
    }

    /// Addresses each item by the source range its value was read from,
    /// byte for byte.
    struct Spans;

    impl SourceAddresser<Range<usize>> for Spans {
        fn source_span(
            &self,
            item: &ExtractedItem<Range<usize>>,
            local: Range<usize>,
        ) -> Vec<SourceRef> {
            let start = item.address.start;
            vec![SourceRef::new(start + local.start..start + local.end)]
        }

        fn locate_source(
            &self,
            items: &[ExtractedItem<Range<usize>>],
            source: &[SourceRef],
        ) -> Vec<ItemEdit> {
            source
                .iter()
                .filter_map(|SourceRef { range, .. }| {
                    let item = items.iter().position(|item| {
                        item.address.start <= range.start && range.end <= item.address.end
                    })?;
                    let start = items[item].address.start;
                    Some(ItemEdit {
                        item,
                        local: range.start - start..range.end - start,
                    })
                })
                .collect()
        }
    }

    /// `{"a":"long secret","b":"Bob"}` as read: its two values, at their
    /// source bytes.
    fn spans() -> ExtractStream<Range<usize>> {
        let item = |value: &str, at: usize| ExtractedItem {
            address: at..at + value.len(),
            value: value.to_owned(),
            hints: Vec::new(),
            passage: Passage::Start,
            flag: None,
        };
        let items = vec![item("long secret", 6), item("Bob", 24)];
        ExtractStream::new(FormatId::new("test"), items, Spans)
    }

    /// After a write, source references still address the source as read:
    /// text a redaction left lifts to its own bytes there, what it wrote to
    /// the bytes it replaced, and a reference to them finds them where they
    /// now read.
    #[tokio::test]
    async fn source_references_address_the_source_as_read_after_a_write() {
        let mut stream = spans();
        stream
            .write_at(Redactions::from([(
                TextLocation::inline(0..4),
                TextReplacement::replaced("X"),
            )]))
            .await
            .unwrap();
        assert_eq!(values_of(&stream), ["X secret", "Bob"]);
        let chunk = stream.chunks().unwrap().remove(0);
        // `secret`, now at 2..8, read at source 11..17.
        let lifted = stream.lift(&chunk, TextLocation::inline(2..8)).unwrap();
        assert_eq!(lifted.source_refs(), [SourceRef::new(11..17)]);
        // `X` stands for all of `long`.
        let lifted = stream.lift(&chunk, TextLocation::inline(0..1)).unwrap();
        assert_eq!(lifted.source_refs(), [SourceRef::new(6..10)]);
        // A reference to `secret`, and one to `ong`, which `X` replaced.
        let mark = |range| TextLocation::marked_source(SourceRef::new(range));
        assert_eq!(stream.resolve(mark(11..17)).unwrap().range(), Some(&(2..8)));
        assert_eq!(stream.resolve(mark(7..10)).unwrap().range(), Some(&(0..1)));
        // A reference to bytes a removal took resolves to where they were, and
        // redacts nothing.
        let mut removed = spans();
        removed
            .write_at(Redactions::from([(
                TextLocation::inline(4..11),
                TextReplacement::Removed,
            )]))
            .await
            .unwrap();
        assert_eq!(values_of(&removed), ["long", "Bob"]);
        assert_eq!(
            removed.resolve(mark(12..15)).unwrap().range(),
            Some(&(4..4))
        );
        removed
            .write_at(Redactions::from([(
                mark(12..15),
                TextReplacement::replaced("Y"),
            )]))
            .await
            .unwrap();
        assert_eq!(values_of(&removed), ["long", "Bob"]);
        // And the next item's references are unmoved by the edit before it.
        assert_eq!(
            stream.resolve(mark(24..27)).unwrap().range(),
            Some(&(8..11))
        );
        stream
            .write_at(Redactions::from([(
                mark(11..17),
                TextReplacement::replaced("Y"),
            )]))
            .await
            .unwrap();
        assert_eq!(values_of(&stream), ["X Y", "Bob"]);
    }

    /// A reference to bytes a removal took at the end of the stream resolves
    /// to the empty span there, and redacts nothing; one past the stream is
    /// still refused.
    #[tokio::test]
    async fn a_removal_at_the_end_of_the_stream_resolves_empty() {
        let item = ExtractedItem {
            address: 6..17,
            value: "long secret".to_owned(),
            hints: Vec::new(),
            passage: Passage::Start,
            flag: None,
        };
        let mut stream = ExtractStream::new(FormatId::new("test"), vec![item], Spans);
        stream
            .write_at(Redactions::from([(
                TextLocation::inline(4..11),
                TextReplacement::Removed,
            )]))
            .await
            .unwrap();
        assert_eq!(values_of(&stream), ["long"]);
        let mark = |range| TextLocation::marked_source(SourceRef::new(range));
        assert_eq!(stream.resolve(mark(12..15)).unwrap().range(), Some(&(4..4)));
        stream
            .write_at(Redactions::from([(
                mark(12..15),
                TextReplacement::replaced("Y"),
            )]))
            .await
            .unwrap();
        assert_eq!(values_of(&stream), ["long"]);
        let past = TextLocation::inline(4..5);
        assert!(stream.read_at(&past).await.is_err());
    }

    fn values_of<A: Send + Sync + 'static>(stream: &ExtractStream<A>) -> Vec<String> {
        stream
            .items()
            .iter()
            .map(|item| item.value.clone())
            .collect()
    }
}
