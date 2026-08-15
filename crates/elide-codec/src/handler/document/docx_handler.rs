//! DOCX handler side: adapts the standalone [`elide_docx`] engine to the
//! codec's [`Handler`] contract.
//!
//! The handler *is* an [`ExtractHandler`] over the text blocks
//! [`Docx::extract`](elide_docx::Docx::extract) recovers from every
//! text-bearing part (body, headers, footers, notes, comments). Each block's
//! [`Address`] is its part plus its byte span, so redaction edits the block
//! value in place; on encode, [`DocxEncoder`] turns the edits into
//! [`elide_docx::block::Replacement`]s (plus any redacted media parts) and calls
//! [`Docx::rewrite_with_parts`](elide_docx::Docx::rewrite_with_parts), which
//! owns the package round-trip.
//!
//! [`ExtractHandler`]: crate::handler::extract::ExtractHandler
//! [`Address`]: crate::handler::extract::Encoder::Address

use std::collections::HashMap;
use std::ops::Range;

use bytes::Bytes;
use elide_core::modality::text::Text;
use elide_core::{Error, ErrorKind, Result};
use elide_docx::block::Embedding;
use elide_docx::part::PartPath;

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
/// [`Docx::extract`](elide_docx::Docx::extract).
#[derive(Debug, Clone)]
pub(crate) struct DocxAddress {
    /// The part the block belongs to.
    pub(crate) part: PartPath,
    /// The block's byte span within the part's XML.
    pub(crate) span: Range<usize>,
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

/// Re-packs a DOCX by delegating to [`Docx::rewrite_with_parts`](elide_docx::Docx::rewrite_with_parts): the
/// redacted body blocks become text replacements and any redacted media parts
/// travel alongside.
#[derive(Debug)]
pub(crate) struct DocxEncoder {
    /// The original package bytes, retained so [`elide_docx`] can re-pack every
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
        // part's XML. `elide_docx` validates and applies these fail-closed.
        let text_replacements: Vec<elide_docx::block::Replacement> = items
            .iter()
            .map(|item| elide_docx::block::Replacement {
                part: item.address.part.clone(),
                start: item.address.span.start,
                end: item.address.span.end,
                text: item.value.clone().into(),
            })
            .collect();
        let media: Vec<elide_docx::block::PartReplacement> = self
            .replacements
            .iter()
            .map(|(name, bytes)| elide_docx::block::PartReplacement {
                part: PartPath::new(name.clone()),
                bytes: bytes.to_vec(),
            })
            .collect();

        let out = elide_docx::Docx::open(&self.archive)
            .and_then(|docx| docx.rewrite_with_parts(&text_replacements, &media))
            .map_err(docx_error)?;
        Ok(ContentData::new(Bytes::from(out)))
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
        let is_embedding = PartPath::new(id.as_str().to_owned())
            .kind()
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

/// Map an [`elide_docx`] error into the codec's error type.
pub(super) fn docx_error(err: elide_docx::Error) -> Error {
    use elide_docx::ErrorKind as DocxKind;
    let kind = match err.kind() {
        DocxKind::InvalidArchive | DocxKind::InvalidPackage | DocxKind::InvalidXml => {
            ErrorKind::MalformedInput
        }
        DocxKind::UnsafeRewrite => ErrorKind::Processing,
        _ => ErrorKind::Processing,
    };
    Error::new(kind, err.to_string())
}
