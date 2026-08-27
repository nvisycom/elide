//! DOCX handler side: adapts the standalone [`elide_office`] engine to the
//! codec's [`Handler`] contract.
//!
//! The handler *is* an [`ExtractHandler`] over the text blocks
//! [`Docx::extract`](elide_office::docx::Docx::extract) recovers from every
//! text-bearing part (body, headers, footers, notes, comments). Each block's
//! [`Address`] is its part plus its byte span, so redaction edits the block
//! value in place; on encode, [`DocxEncoder`] turns the edits into
//! [`elide_office::opc::Replacement`]s (plus any redacted media parts) and calls
//! [`Docx::rewrite_with_parts`](elide_office::docx::Docx::rewrite_with_parts),
//! which owns the package round-trip.
//!
//! [`ExtractHandler`]: crate::handler::extract::ExtractHandler
//! [`Address`]: crate::handler::extract::Encoder::Address

use std::collections::HashMap;
use std::ops::Range;

use bytes::Bytes;
use elide_core::modality::text::{SourceRef, Text};
use elide_core::{Error, ErrorKind, Result};
use elide_office::docx::PartKind;
use elide_office::opc::{Embedding, OffsetMap, PartPath};

use super::DocxLoader;
use crate::codec::{Container, Part, PartId};
use crate::content::ContentData;
use crate::handler::extract::{Encoder, ExtractHandler, ExtractedItem};
use crate::{Format, FormatId};

/// Stable [`FormatId`] for the DOCX codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.docx");

/// Handler type for loaded DOCX content.
pub(crate) type DocxHandler = ExtractHandler<DocxEncoder>;

/// The address of a DOCX text block: which package part it is in, and its byte
/// span within that part's XML, as reported by
/// [`Docx::extract`](elide_office::docx::Docx::extract).
#[derive(Debug, Clone)]
pub(crate) struct DocxAddress {
    /// The part the block belongs to.
    pub(crate) part: PartPath,
    /// The block's byte span within the part's XML.
    pub(crate) span: Range<usize>,
    /// The block's decoded-to-raw offset map, so a byte range into the decoded
    /// value can be translated back to its exact raw source range(s).
    pub(crate) offsets: OffsetMap,
}

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), DocxLoader)
        .with_extensions(["docx"])
        .with_content_types([
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ])
}

/// Re-packs a DOCX by delegating to [`Docx::rewrite_with_parts`](elide_office::docx::Docx::rewrite_with_parts):
/// the redacted body blocks become text replacements and any redacted media
/// parts travel alongside.
#[derive(Debug)]
pub(crate) struct DocxEncoder {
    /// The original package bytes, retained so [`elide_office`] can re-pack every
    /// unredacted part unchanged.
    pub(super) archive: Bytes,
    /// The binary embeddings surfaced for redaction, cached at decode so the
    /// [`Container`] surface lists them and [`replace_part`](Container::replace_part)
    /// validates ids without re-extracting the archive.
    ///
    /// [`Container`]: crate::codec::Container
    pub(super) embeddings: Vec<Embedding>,
    /// Redacted replacements for media parts, keyed by zip entry name, filled
    /// through the [`Container`] surface.
    ///
    /// [`Container`]: crate::codec::Container
    pub(super) replacements: HashMap<String, Bytes>,
}

impl Encoder for DocxEncoder {
    type Address = DocxAddress;

    fn encode(&self, items: &[ExtractedItem<DocxAddress>]) -> Result<ContentData> {
        // Each item's (current) value overwrites its source byte span in its
        // part's XML. `elide_office` validates and applies these fail-closed.
        let text_replacements: Vec<elide_office::opc::Replacement> = items
            .iter()
            .map(|item| elide_office::opc::Replacement {
                part: item.address.part.clone(),
                start: item.address.span.start,
                end: item.address.span.end,
                text: item.value.clone().into(),
            })
            .collect();
        let media: Vec<elide_office::opc::PartReplacement> = self
            .replacements
            .iter()
            .map(|(name, bytes)| elide_office::opc::PartReplacement {
                part: PartPath::new(name.clone()),
                bytes: bytes.to_vec(),
            })
            .collect();

        let out = elide_office::docx::Docx::open(&self.archive)
            .and_then(|docx| docx.rewrite_with_parts(&text_replacements, &media))
            .map_err(docx_error)?;
        Ok(ContentData::new(Bytes::from(out)))
    }

    fn source_span(
        &self,
        item: &ExtractedItem<DocxAddress>,
        local: Range<usize>,
    ) -> Vec<SourceRef> {
        // Translate the decoded-value range back to its raw source range(s) via
        // the block's offset map (already part-absolute), tagging each with the
        // part it lives in. A range crossing an entity yields several runs.
        super::opc_source::source_span(item.address.part.as_str(), &item.address.offsets, local)
    }

    fn locate_source(
        &self,
        items: &[ExtractedItem<DocxAddress>],
        source: &[SourceRef],
    ) -> Option<(usize, Range<usize>)> {
        super::opc_source::locate_source(
            items.iter().map(|item| {
                (
                    item.address.part.as_str(),
                    item.address.span.clone(),
                    &item.address.offsets,
                )
            }),
            source,
        )
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        Some(self)
    }
}

impl Container for DocxEncoder {
    fn parts(&self) -> Vec<Part> {
        // Surface every binary embedding the engine classifies — images
        // (`word/media/`), embedded objects (`word/embeddings/`), and fonts
        // (`word/fonts/`) — from the set cached at decode.
        self.embeddings
            .iter()
            .map(|embedding| {
                let name = embedding.part.as_str().to_owned();
                let hint = name
                    .rsplit_once('.')
                    .map(|(_, e)| e.to_owned())
                    .unwrap_or_default();
                Part {
                    id: name.into(),
                    bytes: embedding.bytes.clone(),
                    hint,
                }
            })
            .collect()
    }

    fn replace_part(&mut self, id: &PartId, bytes: Bytes) -> Result<()> {
        // Reject anything that isn't a binary embedding so a caller can't
        // smuggle bytes into a text/structure part through this surface.
        let is_embedding = PartKind::of(&PartPath::from(id.as_str()))
            .embedding()
            .is_some();
        if !is_embedding {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("docx replace_part: `{id}` is not an embedded media part"),
            ));
        }
        // And reject ids that name no embedding the document actually carries,
        // validated against the set cached at decode — an unknown id must not
        // be silently stored and dropped on rewrite.
        let is_known = self
            .embeddings
            .iter()
            .any(|embedding| embedding.part.as_str() == id.as_str());
        if !is_known {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("docx replace_part: `{id}` is not a known embedded media part"),
            ));
        }
        self.replacements.insert(id.as_str().to_owned(), bytes);
        Ok(())
    }
}

/// Map an [`elide_office`] error into the codec's error type.
pub(super) fn docx_error(err: elide_office::Error) -> Error {
    use elide_office::ErrorKind as DocxKind;
    let kind = match err.kind() {
        DocxKind::InvalidArchive | DocxKind::InvalidPackage | DocxKind::InvalidXml => {
            ErrorKind::MalformedInput
        }
        DocxKind::UnsafeRewrite => ErrorKind::Processing,
        _ => ErrorKind::Processing,
    };
    Error::new(kind, err.to_string())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use elide_core::modality::text::{SourceRef, TextLocation};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;
    use crate::content::ContentData;
    use crate::handler::document::DocxLoader;
    use crate::{Handler, Loader};

    const BODY_PART: &str = "word/document.xml";

    /// A minimal one-part `.docx` whose body carries `body_text` in a `w:t` run.
    fn docx_with_body(body_text: &str) -> ContentData {
        let body = format!(
            r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>{body_text}</w:t></w:r></w:p></w:body></w:document>"#
        );
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut put = |name: &str, bytes: &[u8]| {
            zip.start_file(name, opts).unwrap();
            zip.write_all(bytes).unwrap();
        };
        put("[Content_Types].xml", br#"<?xml version="1.0"?><Types/>"#);
        put("_rels/.rels", br#"<?xml version="1.0"?><Relationships/>"#);
        put(BODY_PART, body.as_bytes());
        ContentData::new(zip.finish().unwrap().into_inner().into())
    }

    /// Read chunks until the one whose decoded text equals `value`.
    async fn chunk_for(
        handler: &mut DocxHandler,
        value: &str,
    ) -> elide_core::modality::Chunk<Text> {
        loop {
            let chunk = handler.read_next().await.unwrap().unwrap();
            if chunk.data.as_str() == value {
                break chunk;
            }
        }
    }

    #[tokio::test]
    async fn source_span_maps_a_finding_across_an_entity_including_its_raw_bytes() {
        // The body text is `Alice &amp; Bob`, decoded to `Alice & Bob`. A finding
        // over the whole decoded text must point back at the raw bytes including
        // the `&amp;` — one contiguous raw range, entity bytes and all.
        let raw = r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Alice &amp; Bob</w:t></w:r></w:p></w:body></w:document>"#;
        let mut handler = DocxLoader
            .decode(docx_with_body("Alice &amp; Bob"))
            .await
            .unwrap();
        let chunk = chunk_for(&mut handler, "Alice & Bob").await;

        // Decoded "Alice & Bob" is 11 bytes; lift the whole value.
        let lifted = handler
            .lift(&chunk, TextLocation::new(0, 11))
            .expect("in bounds");

        let head = raw.find("Alice").unwrap();
        let tail_end = raw.find("</w:t>").unwrap();
        assert_eq!(
            lifted.source,
            vec![SourceRef::in_part(head..tail_end, BODY_PART)]
        );
        // The single range spans the entity: `&amp;` is inside it, not a hole.
        assert_eq!(&raw[head..tail_end], "Alice &amp; Bob");
    }

    #[tokio::test]
    async fn source_span_of_just_the_decoded_entity_char_is_the_entity_raw() {
        // Redacting only the decoded `&` (offset 6..7) must point at all 5 raw
        // bytes of `&amp;`, never an empty or partial range.
        let raw = r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Alice &amp; Bob</w:t></w:r></w:p></w:body></w:document>"#;
        let mut handler = DocxLoader
            .decode(docx_with_body("Alice &amp; Bob"))
            .await
            .unwrap();
        let chunk = chunk_for(&mut handler, "Alice & Bob").await;

        let lifted = handler
            .lift(&chunk, TextLocation::new(6, 7))
            .expect("in bounds");
        let amp = raw.find("&amp;").unwrap();
        assert_eq!(
            lifted.source,
            vec![SourceRef::in_part(amp..amp + "&amp;".len(), BODY_PART)]
        );
    }

    #[tokio::test]
    async fn source_span_of_a_finding_before_the_entity_is_one_run() {
        let raw = r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Alice &amp; Bob</w:t></w:r></w:p></w:body></w:document>"#;
        let mut handler = DocxLoader
            .decode(docx_with_body("Alice &amp; Bob"))
            .await
            .unwrap();
        let chunk = chunk_for(&mut handler, "Alice & Bob").await;

        // Decoded "Alice" is 0..5, wholly before the entity → a single raw run.
        let lifted = handler
            .lift(&chunk, TextLocation::new(0, 5))
            .expect("in bounds");
        let head = raw.find("Alice").unwrap();
        assert_eq!(
            lifted.source,
            vec![SourceRef::in_part(head..head + 5, BODY_PART)]
        );
    }

    #[tokio::test]
    async fn redacts_an_entity_located_only_by_source() {
        // A review layer adds an entity by selecting text in the part — it can
        // express the raw part byte span but not the decoded-stream `range`. The
        // redaction is located purely by `.source` and must edit the right bytes.
        use elide_core::modality::DataWriter;
        use elide_core::modality::text::TextReplacement;
        use elide_core::operator::Redactions;

        let raw = r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Alice Bob</w:t></w:r></w:p></w:body></w:document>"#;
        let mut handler = DocxLoader
            .decode(docx_with_body("Alice Bob"))
            .await
            .unwrap();

        // The raw span of "Bob" in the part — what a DOM selection yields.
        let bob = raw.find("Bob").unwrap();
        let location = TextLocation::new(0, 0) // no usable decoded range
            .with_source([SourceRef::in_part(bob..bob + 3, BODY_PART)]);

        let mut redactions = Redactions::new();
        redactions.push(location, TextReplacement::substituted("[NAME]"));
        handler.write_at(redactions).await.unwrap();

        // The rebuilt part has "Bob" replaced, "Alice" untouched.
        let out = handler.encode().unwrap();
        let bytes = out.as_bytes();
        // The output is a zip; the body part contains the replacement.
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
        let mut body = String::new();
        {
            use std::io::Read;
            archive
                .by_name(BODY_PART)
                .unwrap()
                .read_to_string(&mut body)
                .unwrap();
        }
        assert!(body.contains("Alice [NAME]"), "body was: {body}");
        assert!(!body.contains("Alice Bob"));
    }
}
