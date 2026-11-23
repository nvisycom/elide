//! JSON handler: the source's keys, string values, and scalars as an
//! [`ExtractStream`], spliced back into the source at their exact positions.
//!
//! The loader lexes the source once into [`JsonItem`]s, each a decoded value
//! (what a recognizer sees) and a [`JsonSpan`] mapping it back onto the source
//! bytes. The extract stream does the rest, as it does for markup: it streams
//! each item as its own chunk, reads and redacts by decoded offset, and lifts
//! a finding to its source bytes. Each write is spliced into the source at
//! once, rewriting only what a redaction changed, so formatting (indentation,
//! key order, whitespace) and every byte outside a redacted stretch, escapes
//! included, stay as written, and the source references of what follows a
//! write address the source as it now stands.

use std::borrow::Cow;
use std::fmt;

use elide_codec::content::ContentData;
use elide_codec::extract::{ExtractStream, Splice, splice};
use elide_codec::{Encode, Format, FormatId, Stream};
use elide_core::Result;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::JsonLoader;
use super::escape::json_escape;
use super::parser::parse_items;
use super::span::{JsonAddresser, JsonSpan};
use crate::control::{self, Stripped};

/// Stable [`FormatId`] for the JSON codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.json");

/// The JSON [`Format`], resolved by `.json` and `application/json`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID, JsonLoader)
        .with_extensions(["json"])
        .with_content_types(["application/json"])
}

/// Handler for loaded JSON content: the item stream over the retained source.
pub(crate) struct JsonHandler {
    /// The keys, string values, and scalars.
    body: ExtractStream<JsonSpan>,
    /// The source the items were read from: as it now stands, every write
    /// spliced in.
    source: String,
}

impl fmt::Debug for JsonHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonHandler")
            .field("items", &self.body.items().len())
            .finish_non_exhaustive()
    }
}

impl JsonHandler {
    /// Lex JSON source into a handler, preserving its formatting verbatim.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the source is not well-formed JSON.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn parse(source: &str) -> Result<Self> {
        let items = parse_items(source)?;
        Ok(Self {
            body: ExtractStream::new(FORMAT_ID, items, JsonAddresser),
            source: source.to_owned(),
        })
    }

    /// This handler with the terminal control sequences each key and string
    /// value decodes to stripped ([`control`]). A changed string is written
    /// again, escaped; the rest of the source stays as written.
    pub(super) fn strip_controls(self) -> Result<Stripped<Self>> {
        let mut removed = 0;
        let mut splices = Vec::new();
        for item in self.body.items().iter().filter(|item| item.address.quoted) {
            let stripped = control::strip(&item.value);
            removed += stripped.removed;
            if let Cow::Owned(value) = stripped.kept {
                splices.push(Splice {
                    raw: item.address.range.clone(),
                    text: json_escape(&value),
                });
            }
        }
        let kept = if splices.is_empty() {
            self
        } else {
            Self::parse(&splice(&self.source, splices)?)?
        };
        Ok(Stripped { kept, removed })
    }
}

impl Stream<Text> for JsonHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        self.body.chunks()
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        self.body.lift(chunk, local)
    }

    fn resolve(&self, location: TextLocation) -> Result<TextLocation> {
        self.body.resolve(location)
    }
}

impl Encode for JsonHandler {
    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::from_text(self.source.clone()))
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for JsonHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<TextData> {
        self.body.read_at(location).await
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for JsonHandler {
    /// The batch is spliced into the source, which is read again, so each
    /// item's map back onto the source describes its value as it now reads.
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        self.body.write_at(redactions).await?;
        let splices = self
            .body
            .items()
            .iter()
            .filter_map(|item| item.address.write_back(&item.value))
            .collect();
        *self = Self::parse(&splice(&self.source, splices)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::modality::ModalityLocation;
    use elide_core::modality::text::{SourceRef, TextReplacement};
    use elide_core::primitive::Color;

    use super::*;

    fn handler(src: &str) -> JsonHandler {
        JsonHandler::parse(src).expect("valid JSON")
    }

    /// The first chunk whose decoded value equals `value`.
    fn find_chunk(h: &JsonHandler, value: &str) -> Chunk<Text> {
        h.chunks()
            .unwrap()
            .into_iter()
            .find(|c| c.data.as_str() == value)
            .expect("chunk")
    }

    fn encoded(h: &JsonHandler) -> String {
        h.encode().unwrap().decode().unwrap()
    }

    #[tokio::test]
    async fn stream_yields_keys_and_values_in_order() -> Result<()> {
        let h = handler(r#"{"name":"Alice","age":30}"#);
        let chunks: Vec<String> = h
            .chunks()?
            .into_iter()
            .map(|c| c.data.as_str().to_owned())
            .collect();
        assert_eq!(chunks, vec!["name", "Alice", "age", "30"]);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_values_get_distinct_offsets() -> Result<()> {
        let h = handler(r#"{"a":"same","b":"same"}"#);
        let mut offsets = Vec::new();
        for c in h.chunks()? {
            if c.data.as_str() == "same" {
                offsets.push(c.location.range().unwrap().start);
            }
        }
        assert_eq!(offsets.len(), 2);
        assert_ne!(offsets[0], offsets[1]);
        Ok(())
    }

    /// A range inside a value reads that span, not the whole value: the
    /// anonymizer computes the replacement from what it reads.
    #[tokio::test]
    async fn read_returns_the_span_within_a_value() -> Result<()> {
        let h = handler(r#"{"note":"Contact: a@b.test"}"#);
        let chunk = find_chunk(&h, "Contact: a@b.test");
        let start = chunk.location.range().unwrap().start + "Contact: ".len();
        let email = TextLocation::inline(start..start + "a@b.test".len());
        assert_eq!(h.read_at(&email).await?.as_str(), "a@b.test");
        Ok(())
    }

    /// Offsets are rebuilt after a write that changes a value's length.
    #[tokio::test]
    async fn a_write_moves_the_later_chunks() -> Result<()> {
        let mut h = handler(r#"{"a":"long value","b":"next"}"#);
        let first = find_chunk(&h, "long value");
        h.write_at(Redactions::from([(
            first.location.clone(),
            TextReplacement::replaced("x"),
        )]))
        .await?;
        let next = find_chunk(&h, "next");
        assert_eq!(h.read_at(&next.location).await?.as_str(), "next");
        assert_eq!(encoded(&h), r#"{"a":"x","b":"next"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn read_returns_string() -> Result<()> {
        let h = handler(r#"{"name":"Alice"}"#);
        let mut found = false;
        for chunk in h.chunks()? {
            if h.read_at(&chunk.location).await?.as_str() == "Alice" {
                found = true;
            }
        }
        assert!(found);
        Ok(())
    }

    #[test]
    fn encode_preserves_source_compact() -> Result<()> {
        let src = r#"{"a":1}"#;
        assert_eq!(encoded(&handler(src)), src);
        Ok(())
    }

    #[test]
    fn encode_preserves_source_pretty() -> Result<()> {
        let src = "{\n  \"a\": 1\n}\n";
        assert_eq!(encoded(&handler(src)), src);
        Ok(())
    }

    #[tokio::test]
    async fn redact_whole_string_value() -> Result<()> {
        let mut h = handler(r#"{"name":"Alice"}"#);
        let chunk = find_chunk(&h, "Alice");
        h.write_at(Redactions::from([(
            chunk.location.clone(),
            TextReplacement::replaced("Bob"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"name":"Bob"}"#);
        Ok(())
    }

    /// A covered value is written as one mark per character; a covered
    /// number becomes a string of marks, so the document stays valid JSON.
    #[tokio::test]
    async fn a_covered_value_is_marked_per_character() -> Result<()> {
        let mut h = handler(r#"{"name":"Alice","age":42}"#);
        let name = find_chunk(&h, "Alice");
        let age = find_chunk(&h, "42");
        h.write_at(Redactions::from([
            (
                name.location.clone(),
                TextReplacement::covered(Color::BLACK),
            ),
            (age.location.clone(), TextReplacement::covered(Color::BLACK)),
        ]))
        .await?;
        assert_eq!(encoded(&h), r#"{"name":"█████","age":"██"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_partial_leaf_in_compact_source() -> Result<()> {
        let src = r#"{"email":"alice@example.com"}"#;
        let mut h = handler(src);
        let chunk = find_chunk(&h, "alice@example.com");
        // Decoded-stream offsets: the chunk's decoded start plus the value-local
        // range of "alice" (offset 0 in the value here).
        let value = chunk.data.as_str();
        let at = value.find("alice").unwrap();
        h.write_at(Redactions::from([(
            TextLocation::inline(
                chunk.location.range().unwrap().start + at
                    ..chunk.location.range().unwrap().start + at + "alice".len(),
            ),
            TextReplacement::replaced("[USER]"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"email":"[USER]@example.com"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_partial_leaf_with_escapes() -> Result<()> {
        // decoded value: foo"bar; source: "foo\"bar", redacting "bar" in the
        // decoded stream splices back over the source bytes past the `\"` escape.
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = find_chunk(&h, r#"foo"bar"#);
        let value = chunk.data.as_str();
        let at = value.find("bar").unwrap();
        h.write_at(Redactions::from([(
            TextLocation::inline(
                chunk.location.range().unwrap().start + at
                    ..chunk.location.range().unwrap().start + at + "bar".len(),
            ),
            TextReplacement::replaced("XXX"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"msg":"foo\"XXX"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn u_escape_decodes_in_the_value_and_round_trips() -> Result<()> {
        // `é` is one `é` in the value but six source bytes; a
        // `😀` surrogate pair is one `😀` from twelve source bytes.
        // (Sources built at runtime so the literal backslash-u is unambiguous.)
        let e_acute = "\\u00e9";
        let grin = "\\uD83D\\uDE00";
        let src = format!("{{\"a\":\"caf{e_acute}\",\"b\":\"{grin}\"}}");
        let h = handler(&src);
        let values: Vec<String> = h
            .chunks()?
            .into_iter()
            .map(|c| c.data.as_str().to_owned())
            .collect();
        assert!(values.contains(&"caf\u{e9}".to_owned()), "got {values:?}");
        assert!(values.contains(&"\u{1F600}".to_owned()), "got {values:?}");
        // No redaction: the escapes are spliced back verbatim, never decoded.
        assert_eq!(encoded(&h), src);
        Ok(())
    }

    #[tokio::test]
    async fn redact_a_span_after_a_u_escape() -> Result<()> {
        let src = format!("{{\"msg\":\"caf{} bar\"}}", "\\u00e9");
        let mut h = handler(&src);
        let chunk = find_chunk(&h, "caf\u{e9} bar");
        // Decoded-stream offsets: in the value `café bar`, `caf` is bytes 0..3,
        // `é` is 3..5 (2 UTF-8 bytes), the space is 5, so `bar` starts at value
        // byte 6. The decoded location addresses it directly; write_at maps back
        // across the 6-byte `é` source escape when it splices.
        let value = chunk.data.as_str();
        let at = value.find("bar").unwrap();
        h.write_at(Redactions::from([(
            TextLocation::inline(
                chunk.location.range().unwrap().start + at
                    ..chunk.location.range().unwrap().start + at + "bar".len(),
            ),
            TextReplacement::replaced("XXX"),
        )]))
        .await?;
        // Only `bar` is replaced; the `é` escape before it is spliced through
        // verbatim (the redaction edits `serialized` in place, not a re-render of
        // the decoded value).
        assert_eq!(encoded(&h), format!("{{\"msg\":\"caf{} XXX\"}}", "\\u00e9"));
        Ok(())
    }

    #[tokio::test]
    async fn redacts_a_leaf_located_only_by_source() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A caller with only raw coordinates (no decoded range) locates the value
        // by its raw byte span in the serialized JSON. `bar` sits after the 6-byte
        // `é` escape, so its raw span differs from any decoded offset.
        let src = format!("{{\"msg\":\"caf{} bar\"}}", "\\u00e9");
        let mut h = handler(&src);
        let raw_at = src.find("bar").unwrap();
        let location = TextLocation::marked_source(SourceRef::new(raw_at..raw_at + "bar".len()));
        h.write_at(Redactions::from([(
            location,
            TextReplacement::replaced("XXX"),
        )]))
        .await?;
        assert_eq!(encoded(&h), format!("{{\"msg\":\"caf{} XXX\"}}", "\\u00e9"));
        Ok(())
    }

    /// A source-byte mark resolves to the decoded span of what it covers, so
    /// it reads like a detected entity.
    #[tokio::test]
    async fn a_mark_resolves_to_its_decoded_span() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        let src = format!("{{\"msg\":\"caf{} bar\"}}", "\\u00e9");
        let h = handler(&src);
        let raw_at = src.find("bar").unwrap();
        let mark = TextLocation::marked_source(SourceRef::new(raw_at..raw_at + 3));
        let resolved = h.resolve(mark)?;
        assert_eq!(h.read_at(&resolved).await?.as_str(), "bar");
        let structural = TextLocation::marked_source(SourceRef::new(0..1));
        assert!(h.resolve(structural).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn source_span_covering_the_whole_quoted_token_redacts_the_value() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A caller pointing at the *entire* `"bcd"` token, quotes included, must
        // resolve to the whole value (opening quote -> 0, closing -> value end),
        // not fail with a misleading "splits an escape".
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let open = src.find(r#""bcd""#).unwrap();
        let close = open + r#""bcd""#.len();
        let location = TextLocation::marked_source(SourceRef::new(open..close));
        h.write_at(Redactions::from([(
            location,
            TextReplacement::replaced("X"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"a":"X"}"#);
        Ok(())
    }

    /// A source reference that splits a character, written literally or as
    /// an escape, addresses no value of its own and is rejected.
    #[tokio::test]
    async fn a_source_ref_splitting_a_character_is_rejected() -> Result<()> {
        // A literal `é` (2 UTF-8 bytes), and the 6-byte escape `\u00e9`.
        for src in [
            "{\"a\":\"caf\u{e9}\"}".to_owned(),
            format!("{{\"a\":\"caf{}\"}}", "\\u00e9"),
        ] {
            let mut h = handler(&src);
            let e = src.find("caf").unwrap() + 3;
            let location = TextLocation::marked_source(SourceRef::new(e..e + 1));
            let err = h
                .write_at(Redactions::from([(
                    location,
                    TextReplacement::replaced("X"),
                )]))
                .await
                .expect_err("half a character");
            assert_eq!(err.kind(), ErrorKind::MalformedInput, "{src}");
            assert_eq!(encoded(&h), src);
        }
        Ok(())
    }

    /// A source reference over a whole escape redacts the character it
    /// stands for.
    #[tokio::test]
    async fn a_source_ref_over_a_whole_escape_redacts_its_character() -> Result<()> {
        let src = format!("{{\"a\":\"caf{}\"}}", "\\u00e9");
        let mut h = handler(&src);
        let e = src.find("\\u").unwrap();
        let location = TextLocation::marked_source(SourceRef::new(e..e + 6));
        h.write_at(Redactions::from([(
            location,
            TextReplacement::replaced("e"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"a":"cafe"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn a_part_tagged_source_ref_is_rejected_for_json() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // JSON is single-file; a part-tagged reference does not address it. The
        // caller supplied an explicit source coordinate to redact, so an
        // unresolvable one is an error, not a silent no-op that would leave a
        // green audit over an unredacted document.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let at = src.find("bcd").unwrap();
        let location =
            TextLocation::marked_source(SourceRef::in_part(at..at + 3, "some/part.json"));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("part-tagged ref must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn a_source_ref_landing_in_structural_bytes_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // Offset 0 is the opening `{`, structural, no string value there. An
        // explicit source that resolves to no leaf must error, not silently
        // drop the edit.
        let mut h = handler(r#"{"a":"b"}"#);
        let location = TextLocation::marked_source(SourceRef::new(0..1));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("structural source must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn a_source_ref_past_the_document_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A source span past every slot resolves to nothing, a caller mistake
        // (e.g. an off-by-one in a run→byte mapping), so it errors.
        let src = r#"{"a":"b"}"#;
        let mut h = handler(src);
        let past = src.len() + 4;
        let location = TextLocation::marked_source(SourceRef::new(past..past + 2));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("out-of-range source must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn every_source_ref_of_a_multi_ref_location_is_resolved() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A location whose source was fused from several runs carries several
        // refs. Both must resolve and the edit spans their union, resolving only
        // the first would leave the rest of the caller's span unredacted.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let location =
            TextLocation::marked_sources([SourceRef::new(b..b + 1), SourceRef::new(b + 1..b + 3)])
                .unwrap();
        h.write_at(Redactions::from([(
            location,
            TextReplacement::replaced("X"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"a":"X"}"#);
        Ok(())
    }

    /// A detection fused with a mark carries only the mark's reference, but
    /// its span is the whole value: the write covers what the read covers.
    #[tokio::test]
    async fn a_cluster_of_a_span_and_a_mark_redacts_the_whole_span() -> Result<()> {
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let mark = h.resolve(TextLocation::marked_source(SourceRef::new(b..b + 1)))?;
        let start = mark.range().unwrap().start;
        let fused = TextLocation::inline(start..start + 3).union(&mark).unwrap();
        assert_eq!(fused.source_refs(), [SourceRef::new(b..b + 1)]);
        assert_eq!(h.read_at(&fused).await?.as_str(), "bcd");
        h.write_at(Redactions::from([(fused, TextReplacement::Removed)]))
            .await?;
        assert_eq!(encoded(&h), r#"{"a":""}"#);
        Ok(())
    }

    /// Refs with text between them are refused: one edit over both would
    /// take that text too.
    #[tokio::test]
    async fn source_refs_with_text_between_them_are_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let location =
            TextLocation::marked_sources([SourceRef::new(b..b + 1), SourceRef::new(b + 2..b + 3)])
                .unwrap();
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("`c` lies between the refs");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src);
        Ok(())
    }

    #[tokio::test]
    async fn a_valid_first_ref_with_an_invalid_second_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // The first ref resolves cleanly; the second is out of range. The whole
        // location must be rejected, never splice the first and report success
        // while dropping the second, which would leave that coordinate unredacted
        // under a green audit.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let past = src.len() + 4;
        let location = TextLocation::marked_sources([
            SourceRef::new(b..b + 1),
            SourceRef::new(past..past + 2),
        ])
        .unwrap();
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("an invalid trailing ref must reject the whole location");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        // And nothing was spliced: the document is untouched.
        assert_eq!(encoded(&h), src, "no partial redaction on rejection");
        Ok(())
    }

    #[tokio::test]
    async fn source_refs_spanning_two_json_values_are_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // Two refs that each resolve, but to different leaves. Aggregating a span
        // across two values is meaningless, so it errors rather than silently
        // redacting just one.
        let src = r#"{"a":"bcd","e":"fgh"}"#;
        let mut h = handler(src);
        let b = src.find("bcd").unwrap();
        let f = src.find("fgh").unwrap();
        let location =
            TextLocation::marked_sources([SourceRef::new(b..b + 1), SourceRef::new(f..f + 1)])
                .unwrap();
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("refs across two values must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "no partial redaction on rejection");
        Ok(())
    }

    #[tokio::test]
    async fn a_source_ref_of_only_the_opening_quote_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A span covering just the opening `"` of `"bcd"` addresses no value
        // byte. Its endpoints would collapse to value offset 0 and `splice`
        // would insert `X` at the value start rather than redact, reject it.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let open = src.find(r#""bcd""#).unwrap();
        let location = TextLocation::marked_source(SourceRef::new(open..open + 1));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("a delimiter-only span must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "document unchanged");
        Ok(())
    }

    #[tokio::test]
    async fn a_source_ref_of_only_the_closing_quote_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // The closing `"` of `"bcd"`. Its endpoints collapse to the value end,
        // where `splice` would append `X`, reject rather than corrupt.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let close = src.find(r#""bcd""#).unwrap() + r#""bcd""#.len() - 1;
        let location = TextLocation::marked_source(SourceRef::new(close..close + 1));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("a delimiter-only span must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "document unchanged");
        Ok(())
    }

    #[tokio::test]
    async fn an_empty_source_ref_inside_a_value_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A zero-width span in the interior of `"bcd"` (source `7..7`, at `c`).
        // Both endpoints map to one value offset, so `splice` would insert `X`
        // (yielding `bXcd`) rather than redact, reject it.
        let src = r#"{"a":"bcd"}"#;
        let mut h = handler(src);
        let c = src.find("bcd").unwrap() + 1;
        let location = TextLocation::marked_source(SourceRef::new(c..c));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("an empty interior span must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "document unchanged");
        Ok(())
    }

    #[tokio::test]
    async fn an_empty_source_ref_inside_a_scalar_is_rejected() -> Result<()> {
        use elide_core::modality::text::SourceRef;

        // A scalar's source→value mapping is identity, so a zero-width span in it
        // has the same insert-not-redact gap. Reject it too.
        let src = r#"{"a":12345}"#;
        let mut h = handler(src);
        let mid = src.find("12345").unwrap() + 2;
        let location = TextLocation::marked_source(SourceRef::new(mid..mid));
        let err = h
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("an empty scalar span must be rejected");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src, "document unchanged");
        Ok(())
    }

    #[tokio::test]
    async fn the_decoded_stream_holds_only_keys_and_values() -> Result<()> {
        // Structure is not text: offset 0 is the first key, not the `{`, and
        // the structure around a redaction stays as written.
        let mut h = handler(r#"{ "a" : "b" }"#);
        h.write_at(Redactions::from([(
            TextLocation::inline(0..1),
            TextReplacement::replaced("X"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{ "X" : "b" }"#);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_range_past_its_value() -> Result<()> {
        // A span that starts in the "b" value but runs past its end into the
        // closing quote / brace is rejected with a named error, not a late
        // generic splice failure.
        let mut h = handler(r#"{"a":"b"}"#);
        let chunk = find_chunk(&h, "b");
        let start = chunk.location.range().unwrap().start;
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(start..start + 5),
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_range_splitting_a_utf8_char() -> Result<()> {
        // In the value `café`, `é` is bytes 3..5; a range ending at 4 splits it.
        // The rejection must happen before any mutation, so the document is
        // left byte-identical.
        let src = format!("{{\"a\":\"caf{}\"}}", "\\u00e9");
        let mut h = handler(&src);
        let chunk = find_chunk(&h, "caf\u{e9}");
        let start = chunk.location.range().unwrap().start;
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(start + 3..start + 4),
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        // No partial mutation: the source is untouched after the rejected batch.
        assert_eq!(encoded(&h), src);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_reversed_range() -> Result<()> {
        // A reversed range (start > end) must be rejected before the value-local
        // `end - slot_offset` can underflow-panic in a checked build.
        let mut h = handler(r#"{"a":"bcd"}"#);
        let chunk = find_chunk(&h, "bcd");
        let start = chunk.location.range().unwrap().start;
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(start + 2..start),
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_overlapping_edits_of_one_value() -> Result<()> {
        let src = r#"{"a":"Jane Roe"}"#;
        let mut h = handler(src);
        let start = find_chunk(&h, "Jane Roe").location.range().unwrap().start;
        let err = h
            .write_at(Redactions::from([
                (
                    TextLocation::inline(start..start + 6),
                    TextReplacement::replaced("[A]"),
                ),
                (
                    TextLocation::inline(start + 5..start + 8),
                    TextReplacement::replaced("[B]"),
                ),
            ]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(encoded(&h), src);
        // Touching edits are not an overlap.
        h.write_at(Redactions::from([
            (
                TextLocation::inline(start..start + 5),
                TextReplacement::replaced("[A]"),
            ),
            (
                TextLocation::inline(start + 5..start + 8),
                TextReplacement::replaced("[B]"),
            ),
        ]))
        .await?;
        assert_eq!(encoded(&h), r#"{"a":"[A][B]"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn read_at_rejects_a_reversed_range() -> Result<()> {
        // An end before the value's start would underflow its local offset.
        let h = handler(r#"{"a":"bcd"}"#);
        let chunk = find_chunk(&h, "bcd");
        let start = chunk.location.range().unwrap().start;
        let err = h
            .read_at(&TextLocation::inline(start + 2..0))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn write_at_rejects_a_range_at_the_document_end() -> Result<()> {
        // A start at (or past) the decoded-stream end resolves to no leaf; the
        // sweep must reject it rather than exhaust silently and return Ok.
        let mut h = handler(r#"{"a":"b"}"#);
        let decoded_len = h.chunks()?.last().unwrap().location.range().unwrap().end;
        let err = h
            .write_at(Redactions::from([(
                TextLocation::inline(decoded_len..decoded_len + 1),
                TextReplacement::replaced("X"),
            )]))
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        Ok(())
    }

    #[tokio::test]
    async fn redact_key() -> Result<()> {
        let mut h = handler(r#"{"email":"a@b.c"}"#);
        let chunk = find_chunk(&h, "email");
        h.write_at(Redactions::from([(
            chunk.location.clone(),
            TextReplacement::replaced("contact"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"contact":"a@b.c"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_scalar() -> Result<()> {
        let mut h = handler(r#"{"n":42}"#);
        let chunk = find_chunk(&h, "42");
        h.write_at(Redactions::from([(
            chunk.location.clone(),
            TextReplacement::replaced("0"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"n":0}"#);
        Ok(())
    }

    #[tokio::test]
    async fn redact_scalar_with_non_literal_replacement_quotes_it() -> Result<()> {
        // Masking a number with a non-literal string would make bare `XXX`
        // invalid JSON, so the leaf is promoted to a quoted string value.
        let mut h = handler(r#"{"n":42}"#);
        let chunk = find_chunk(&h, "42");
        h.write_at(Redactions::from([(
            chunk.location.clone(),
            TextReplacement::replaced("XXX"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"n":"XXX"}"#);
        Ok(())
    }

    /// Multiple redactions in a single batch, each targeting a different
    /// leaf, with length deltas that shift later slot offsets. Regression
    /// test for the "only the first redaction lands" bug.
    #[tokio::test]
    async fn redact_multiple_leaves_with_shifting_offsets() -> Result<()> {
        let src = r#"{"a":"first","b":"second","c":"third"}"#;
        let mut h = handler(src);
        let mut locs = Vec::new();
        for c in h.chunks()? {
            let v = c.data.as_str();
            if v == "first" || v == "second" || v == "third" {
                locs.push(c.location);
            }
        }
        assert_eq!(locs.len(), 3, "expected three string values");
        let rs = locs
            .into_iter()
            .map(|loc| (loc, TextReplacement::replaced("X")))
            .collect();
        h.write_at(rs).await?;
        assert_eq!(encoded(&h), r#"{"a":"X","b":"X","c":"X"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn lift_simple_string() -> Result<()> {
        // No escapes: decoded and source coincide, so `range` and `.source`
        // agree, but `.source` is still populated with the raw span.
        let src = r#"{"email":"alice@example.com"}"#;
        let h = handler(src);
        let chunk = find_chunk(&h, "alice@example.com");
        let value_start = "alice@example.com".find("alice").unwrap();
        let value_end = value_start + "alice".len();
        let lifted = h
            .lift(&chunk, TextLocation::inline(value_start..value_end))
            .expect("range is in bounds");
        // Primary range: decoded-stream offset (chunk start + value-local).
        let lifted_range = lifted.range().unwrap();
        let chunk_start = chunk.location.range().unwrap().start;
        assert_eq!(lifted_range.start, chunk_start + value_start);
        assert_eq!(lifted_range.end, chunk_start + value_end);
        // Source: the raw byte span of "alice" in the document.
        let raw = src.find("alice").unwrap();
        assert_eq!(
            lifted.source_refs(),
            &[SourceRef::new(raw..raw + "alice".len())][..]
        );
        Ok(())
    }

    #[tokio::test]
    async fn lift_walks_escapes() -> Result<()> {
        // decoded value: foo"bar; source: "foo\"bar". The `\"` escape makes the
        // decoded stream one byte shorter than the source before "bar", so the
        // decoded `range` and the raw `.source` diverge, this is the case the
        // whole fix exists for.
        let src = r#"{"msg":"foo\"bar"}"#;
        let h = handler(src);
        let chunk = find_chunk(&h, r#"foo"bar"#);
        let value = chunk.data.as_str();
        let value_start = value.find("bar").unwrap();
        let value_end = value_start + "bar".len();
        let lifted = h
            .lift(&chunk, TextLocation::inline(value_start..value_end))
            .expect("range is in bounds");
        // Decoded range: `bar` at value offset 4 (`foo"` = 4 decoded bytes).
        let lifted_range = lifted.range().unwrap();
        let chunk_start = chunk.location.range().unwrap().start;
        assert_eq!(lifted_range.start, chunk_start + value_start);
        assert_eq!(lifted_range.end, chunk_start + value_end);
        // Raw source span: `bar` sits after the 2-byte `\"` escape.
        let raw = src.find("bar").unwrap();
        assert_eq!(
            lifted.source_refs(),
            &[SourceRef::new(raw..raw + "bar".len())][..]
        );
        Ok(())
    }

    /// After a write, a finding lifts to the source bytes it reads as now,
    /// and a source reference to them redacts it.
    #[tokio::test]
    async fn source_references_follow_a_write() -> Result<()> {
        let mut h = handler(r#"{"a":"long secret"}"#);
        let chunk = find_chunk(&h, "long secret");
        let start = chunk.location.range().unwrap().start;
        h.write_at(Redactions::from([(
            TextLocation::inline(start..start + 4),
            TextReplacement::replaced("X"),
        )]))
        .await?;
        let src = encoded(&h);
        assert_eq!(src, r#"{"a":"X secret"}"#);
        let chunk = find_chunk(&h, "X secret");
        let lifted = h.lift(&chunk, TextLocation::inline(2..8)).unwrap();
        let raw = src.find("secret").unwrap();
        assert_eq!(lifted.source_refs(), [SourceRef::new(raw..raw + 6)]);
        let mark = TextLocation::marked_source(SourceRef::new(raw..raw + 6));
        h.write_at(Redactions::from([(mark, TextReplacement::replaced("Y"))]))
            .await?;
        assert_eq!(encoded(&h), r#"{"a":"X Y"}"#);
        Ok(())
    }

    #[tokio::test]
    async fn lift_redact_roundtrip() -> Result<()> {
        let src = r#"{"msg":"foo\"bar"}"#;
        let mut h = handler(src);
        let chunk = find_chunk(&h, r#"foo"bar"#);
        let value = chunk.data.as_str();
        let value_start = value.find("bar").unwrap();
        let value_end = value_start + "bar".len();
        let source_loc = h
            .lift(&chunk, TextLocation::inline(value_start..value_end))
            .expect("range is in bounds");
        h.write_at(Redactions::from([(
            source_loc,
            TextReplacement::replaced("XXX"),
        )]))
        .await?;
        assert_eq!(encoded(&h), r#"{"msg":"foo\"XXX"}"#);
        Ok(())
    }
}
