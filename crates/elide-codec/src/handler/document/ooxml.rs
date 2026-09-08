//! The shared codec adapter for the text-only OOXML formats (DOCX, PPTX).
//!
//! Both open to the same [`elide_office::ooxml::OoxmlPackage`] and adapt to the
//! codec's [`Encoder`] / [`Container`] contract identically: extract element-text
//! blocks addressed by `(part, span, OffsetMap)`, surface embeddings and
//! document-property parts as sub-parts, and re-pack via
//! [`rewrite_with_parts`](elide_office::ooxml::OoxmlPackage::rewrite_with_parts).
//! The only per-format differences, the engine format, the codec [`FormatId`],
//! and the label used in errors, are named by the [`OoxmlCodec`] seam; everything
//! else lives here once.

use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;

use bytes::Bytes;
use elide_core::modality::text::SourceRef;
use elide_core::{Error, ErrorKind, Result};
use elide_office::ooxml::{OoxmlFormat, OoxmlPackage};
use elide_office::opc::{
    Embedding, OffsetMap, PartClassifier, PartPath, PartReplacement, PartRole, Replacement,
};

use crate::codec::{Container, Part};
use crate::content::ContentData;
use crate::handler::extract::{Encoder, ExtractHandler, ExtractedItem, ItemEdit};
use crate::{FormatId, LocalId};

/// Ties a text-only OOXML [`elide_office`] format to its codec identity: the
/// engine format it opens, the stable [`FormatId`] it registers under, and the
/// short label used in its error messages (`docx`, `pptx`).
pub(crate) trait OoxmlCodec: Send + Sync + 'static {
    /// The [`elide_office`] format this codec drives.
    type Format: OoxmlFormat;

    /// The stable codec [`FormatId`].
    const FORMAT_ID: FormatId;

    /// A short lowercase name for the format, used in error messages.
    const LABEL: &'static str;
}

/// The handler type for a text-only OOXML codec.
pub(crate) type OoxmlHandler<C> = ExtractHandler<OoxmlEncoder<C>>;

/// The address of an OOXML text block: which package part it is in, its byte
/// span within that part's XML, and the decoded-to-raw offset map that maps a
/// range into the decoded value back to its exact raw source range(s).
#[derive(Debug, Clone)]
pub(crate) struct OoxmlAddress {
    /// The part the block belongs to.
    pub(crate) part: PartPath,
    /// The block's byte span within the part's XML.
    pub(crate) span: Range<usize>,
    /// The block's decoded-to-raw offset map.
    pub(crate) offsets: OffsetMap,
}

/// Re-packs an OOXML document by delegating to
/// [`rewrite_with_parts`](OoxmlPackage::rewrite_with_parts): the redacted text
/// blocks become text replacements and any redacted media or property parts
/// travel alongside.
#[derive(Debug)]
pub(crate) struct OoxmlEncoder<C: OoxmlCodec> {
    /// The original package bytes, retained so [`elide_office`] can re-pack every
    /// unredacted part unchanged.
    archive: Bytes,
    /// The binary embeddings surfaced for redaction, cached at decode so the
    /// [`Container`] surface lists them and [`replace_part`](Container::replace_part)
    /// validates ids without re-extracting the archive.
    embeddings: Vec<Embedding>,
    /// The document-property parts (`docProps/core.xml`, `app.xml`) as
    /// `(zip entry name, bytes)`, cached at decode so the [`Container`] surface
    /// lists them as metadata sub-parts.
    doc_props: Vec<(String, Bytes)>,
    /// Redacted replacements for media and property parts, keyed by zip entry
    /// name, filled through the [`Container`] surface.
    replacements: HashMap<String, Bytes>,
    /// Zero-sized: the encoder is generic over the format, but carries no value
    /// of it.
    _codec: std::marker::PhantomData<C>,
}

impl<C: OoxmlCodec> Encoder for OoxmlEncoder<C> {
    type Address = OoxmlAddress;

    fn encode(&self, items: &[ExtractedItem<OoxmlAddress>]) -> Result<ContentData> {
        // Each item's (current) value overwrites its source byte span in its
        // part's XML. `elide_office` validates and applies these fail-closed.
        let text_replacements: Vec<Replacement> = items
            .iter()
            .map(|item| Replacement {
                part: item.address.part.clone(),
                start: item.address.span.start,
                end: item.address.span.end,
                text: item.value.clone().into(),
            })
            .collect();
        let media: Vec<PartReplacement> = self
            .replacements
            .iter()
            .map(|(name, bytes)| PartReplacement::new(PartPath::new(name.clone()), bytes.to_vec()))
            .collect();

        let out = OoxmlPackage::<C::Format>::open(&self.archive)
            .and_then(|pkg| pkg.rewrite_with_parts(&text_replacements, &media))
            .map_err(crate::handler::office::office_error)?;
        Ok(ContentData::new(Bytes::from(out)))
    }

    fn source_span(
        &self,
        item: &ExtractedItem<OoxmlAddress>,
        local: Range<usize>,
    ) -> Vec<SourceRef> {
        // Translate the decoded-value range back to its raw source range(s) via
        // the block's offset map (already part-absolute), tagging each with the
        // part it lives in. A range crossing an entity yields several runs.
        super::opc_source::source_span(item.address.part.as_str(), &item.address.offsets, local)
    }

    fn locate_source(
        &self,
        items: &[ExtractedItem<OoxmlAddress>],
        source: &[SourceRef],
    ) -> Option<ItemEdit> {
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

impl<C: OoxmlCodec> Container for OoxmlEncoder<C> {
    fn parts(&self) -> Vec<Part> {
        // Surface every binary embedding the engine classifies (images, media,
        // embedded objects, fonts), from the set cached at decode.
        let embeddings = self.embeddings.iter().map(|embedding| {
            let id = LocalId::new(embedding.part.as_str().to_owned());
            let hint = id.extension().unwrap_or_default().to_owned();
            Part {
                id,
                bytes: embedding.bytes.clone(),
                hint,
            }
        });
        // Plus each document-property part, decoded as the metadata modality.
        let props = self.doc_props.iter().map(|(path, bytes)| Part {
            id: LocalId::new(path.clone()),
            bytes: bytes.clone(),
            hint: crate::handler::docprops_hint().to_owned(),
        });
        embeddings.chain(props).collect()
    }

    fn replace_part(&mut self, id: &LocalId, bytes: Bytes) -> Result<()> {
        // A replacement may target a binary embedding or a document-property
        // part, both cached at decode. Anything else, a text or structure part,
        // is rejected so a caller can't smuggle bytes past the text-splice path.
        let path = PartPath::from(id.as_str());
        let is_embedding = matches!(C::Format::classifier().role(&path), PartRole::Binary(_))
            && self.embeddings.iter().any(|e| e.part == path);
        let is_property = self.doc_props.iter().any(|(p, _)| p == id.as_str());
        if !is_embedding && !is_property {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!(
                    "{} replace_part: `{id}` is not a known embedded or property part",
                    C::LABEL
                ),
            ));
        }
        self.replacements.insert(id.as_str().to_owned(), bytes);
        Ok(())
    }
}

/// Open the package, extract its text blocks and embeddings, and build the
/// [`OoxmlHandler`]. Shared by the DOCX and PPTX loaders.
///
/// **Fail-closed:** a non-empty [`issues`](elide_office::opc::Extraction::issues)
/// list means a text-bearing part could not be read into blocks, so its text
/// would ship un-redacted; that refuses the decode rather than emitting a
/// partially-extracted document.
///
/// # Errors
///
/// - [`MalformedInput`](ErrorKind::MalformedInput) if the bytes are not a valid
///   package of the format, or a text-bearing part could not be extracted.
pub(crate) fn decode_extract<C: OoxmlCodec>(content: ContentData) -> Result<OoxmlHandler<C>> {
    let archive = content.to_bytes();
    let pkg =
        OoxmlPackage::<C::Format>::open(&archive).map_err(crate::handler::office::office_error)?;
    // Cache the property parts' bytes so the `Container` can surface them as
    // metadata sub-parts without re-opening the package.
    let doc_props = crate::handler::office::props::read_doc_props(|path| pkg.part_bytes(path));
    let extraction = pkg.extract();

    if !extraction.issues.is_empty() {
        let mut parts = String::new();
        for issue in &extraction.issues {
            if !parts.is_empty() {
                parts.push_str(", ");
            }
            let _ = write!(parts, "{} ({:?})", issue.part, issue.kind);
        }
        return Err(Error::new(
            ErrorKind::MalformedInput,
            format!(
                "{}: text-bearing part(s) could not be extracted: {parts}",
                C::LABEL
            ),
        ));
    }

    let items: Vec<ExtractedItem<OoxmlAddress>> = extraction
        .blocks
        .into_iter()
        .map(|block| ExtractedItem {
            value: block.text.to_string(),
            address: OoxmlAddress {
                part: block.part,
                span: block.start..block.end,
                offsets: block.offsets,
            },
            hints: Vec::new(),
        })
        .collect();
    Ok(ExtractHandler::new(
        C::FORMAT_ID,
        OoxmlEncoder {
            archive,
            embeddings: extraction.embeddings,
            doc_props,
            replacements: HashMap::new(),
            _codec: std::marker::PhantomData,
        },
        items,
    ))
}
