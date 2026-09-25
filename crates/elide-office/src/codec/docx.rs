//! DOCX codec: binds the WordprocessingML engine format to the shared
//! [`ooxml`](super::ooxml) codec adapter.
//!
//! Everything but the format identity lives in [`super::ooxml`]: the document is
//! a body [`ExtractStream`](elide_codec::extract::ExtractStream) over the
//! element-text blocks [`Docx::extract`](crate::docx::Docx::extract) recovers
//! plus its embedding / document-property blobs, re-packed via the shared
//! [`OoxmlRecombine`](super::ooxml::OoxmlRecombine).

use elide_codec::{Format, FormatId};

use super::ooxml::{OoxmlCodec, OoxmlLoader};
use crate::docx::WordFormat;

/// The DOCX codec seam: WordprocessingML over the shared OOXML adapter.
#[derive(Debug)]
pub(crate) struct DocxCodec;

impl OoxmlCodec for DocxCodec {
    type Format = WordFormat;

    const FORMAT_ID: FormatId = FormatId::new("elide.document.docx");
    const LABEL: &'static str = "docx";
}

/// Stable [`FormatId`] for the DOCX codec.
pub const FORMAT_ID: FormatId = DocxCodec::FORMAT_ID;

/// [`Format`] descriptor registered into `FormatRegistry`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID.clone(), OoxmlLoader::<DocxCodec>::new())
        .with_extensions(["docx"])
        .with_content_types([
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ])
}

#[cfg(test)]
mod tests {
    use elide_codec::content::ContentData;
    use elide_codec::extract::ExtractStream;
    use elide_codec::{Recombine, Stream};
    use elide_core::modality::text::{SourceRef, Text, TextLocation};

    use super::*;
    use crate::codec::ooxml::{OoxmlAddress, OoxmlRecombine, decode_parts};
    use crate::opc::test_util;

    const BODY_PART: &str = "word/document.xml";

    /// A minimal one-part `.docx` whose body carries `body_text` in a `w:t` run.
    fn docx_with_body(body_text: &str) -> ContentData {
        let body = format!(
            r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>{body_text}</w:t></w:r></w:p></w:body></w:document>"#
        );
        let package = test_util::pack_parts(&[
            ("[Content_Types].xml", br#"<?xml version="1.0"?><Types/>"#),
            ("_rels/.rels", br#"<?xml version="1.0"?><Relationships/>"#),
            (BODY_PART, body.as_bytes()),
        ]);
        ContentData::new(package.into())
    }

    /// Decode `content` into the DOCX body stream and its recombiner.
    fn decode(content: ContentData) -> (ExtractStream<OoxmlAddress>, OoxmlRecombine<DocxCodec>) {
        decode_parts::<DocxCodec>(content).unwrap()
    }

    /// Read chunks until the one whose decoded text equals `value`.
    async fn chunk_for(
        stream: &mut ExtractStream<OoxmlAddress>,
        value: &str,
    ) -> elide_core::modality::Chunk<Text> {
        loop {
            let chunk = stream.read_next().await.unwrap().unwrap();
            if chunk.data.as_str() == value {
                break chunk;
            }
        }
    }

    #[tokio::test]
    async fn source_span_maps_a_finding_across_an_entity_including_its_raw_bytes() {
        // The body text is `Alice &amp; Bob`, decoded to `Alice & Bob`. A finding
        // over the whole decoded text must point back at the raw bytes including
        // the `&amp;`, one contiguous raw range, entity bytes and all.
        let raw = r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Alice &amp; Bob</w:t></w:r></w:p></w:body></w:document>"#;
        let (mut stream, _recombine) = decode(docx_with_body("Alice &amp; Bob"));
        let chunk = chunk_for(&mut stream, "Alice & Bob").await;

        // Decoded "Alice & Bob" is 11 bytes; lift the whole value.
        let lifted = stream
            .lift(&chunk, TextLocation::new(0, 11))
            .expect("in bounds");

        let head = raw.find("Alice").unwrap();
        let tail_end = raw.find("</w:t>").unwrap();
        assert_eq!(
            lifted.source(),
            &[SourceRef::in_part(head..tail_end, BODY_PART)][..]
        );
        // The single range spans the entity: `&amp;` is inside it, not a hole.
        assert_eq!(&raw[head..tail_end], "Alice &amp; Bob");
    }

    #[tokio::test]
    async fn source_span_of_just_the_decoded_entity_char_is_the_entity_raw() {
        // Redacting only the decoded `&` (offset 6..7) must point at all 5 raw
        // bytes of `&amp;`, never an empty or partial range.
        let raw = r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Alice &amp; Bob</w:t></w:r></w:p></w:body></w:document>"#;
        let (mut stream, _recombine) = decode(docx_with_body("Alice &amp; Bob"));
        let chunk = chunk_for(&mut stream, "Alice & Bob").await;

        let lifted = stream
            .lift(&chunk, TextLocation::new(6, 7))
            .expect("in bounds");
        let amp = raw.find("&amp;").unwrap();
        assert_eq!(
            lifted.source(),
            &[SourceRef::in_part(amp..amp + "&amp;".len(), BODY_PART)][..]
        );
    }

    #[tokio::test]
    async fn source_span_of_a_finding_before_the_entity_is_one_run() {
        let raw = r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Alice &amp; Bob</w:t></w:r></w:p></w:body></w:document>"#;
        let (mut stream, _recombine) = decode(docx_with_body("Alice &amp; Bob"));
        let chunk = chunk_for(&mut stream, "Alice & Bob").await;

        // Decoded "Alice" is 0..5, wholly before the entity → a single raw run.
        let lifted = stream
            .lift(&chunk, TextLocation::new(0, 5))
            .expect("in bounds");
        let head = raw.find("Alice").unwrap();
        assert_eq!(
            lifted.source(),
            &[SourceRef::in_part(head..head + 5, BODY_PART)][..]
        );
    }

    #[tokio::test]
    async fn redacts_an_entity_located_only_by_source() {
        // A review layer adds an entity by selecting text in the part, it can
        // express the raw part byte span but not the decoded-stream `range`. The
        // redaction is located purely by `.source` and must edit the right bytes.
        use elide_core::modality::DataWriter;
        use elide_core::modality::text::TextReplacement;
        use elide_core::redaction::Redactions;

        let raw = r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Alice Bob</w:t></w:r></w:p></w:body></w:document>"#;
        let (mut stream, recombine) = decode(docx_with_body("Alice Bob"));

        // The raw span of "Bob" in the part, what a DOM selection yields. The
        // reviewer has no decoded-stream range, only this raw span, so the
        // location is genuinely source-only (no faked `range`).
        let bob = raw.find("Bob").unwrap();
        let location = TextLocation::from_source([SourceRef::in_part(bob..bob + 3, BODY_PART)]);
        assert!(location.is_source_only());

        let mut redactions = Redactions::new();
        redactions.push(location, TextReplacement::substituted("[NAME]"));
        stream.write_at(redactions).await.unwrap();

        // The rebuilt part has "Bob" replaced, "Alice" untouched.
        let out = recombine.assemble(&[]).unwrap();
        // The output is an OPC package; the body part contains the replacement.
        let body_bytes =
            test_util::read_part(out.as_bytes(), BODY_PART).expect("body part present");
        let body = String::from_utf8(body_bytes).expect("body part is UTF-8");
        assert!(body.contains("Alice [NAME]"), "body was: {body}");
        assert!(!body.contains("Alice Bob"));
    }
}
