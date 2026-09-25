//! [`OoxmlLoader`] + [`decode_document`]: build a parts-model [`Document`] from a
//! text-only OOXML package.

use std::marker::PhantomData;
use std::sync::Arc;

use elide_codec::content::ContentData;
use elide_codec::extract::{ExtractStream, ExtractedItem, SharedSplice};
use elide_codec::{Document, DocumentLoader, DocumentPart, ErasedStream, LocalId, Stream};
use elide_core::modality::text::Text;
use elide_core::{Error, ErrorKind, Result};

use super::addresser::OoxmlAddresser;
use super::recombine::OoxmlRecombine;
use super::{BODY_PART_ID, OoxmlAddress, OoxmlCodec};
use crate::ooxml::OoxmlPackage;

/// Open the package, extract its text blocks and embeddings, and build the
/// parts-model [`Document`]: a body [`ExtractStream`] plus a
/// [`Blob`](DocumentPart::Blob) per surfaced embedding and document-property
/// part. Shared by the DOCX and PPTX loaders.
///
/// **Fail-closed:** a non-empty [`issues`](crate::opc::Extraction::issues)
/// list means a text-bearing part could not be read into blocks, so its text
/// would ship un-redacted; that refuses the decode rather than emitting a
/// partially-extracted document.
///
/// Only embeddings the engine classifies as
/// [`Binary`](crate::opc::PartRole::Binary) and the document-property parts are
/// surfaced as blobs; a text or structure part is never exposed, so it can't be
/// clobbered past the text-splice path.
///
/// # Errors
///
/// - [`MalformedInput`](ErrorKind::MalformedInput) if the bytes are not a valid
///   package of the format, or a text-bearing part could not be extracted.
pub(crate) fn decode_document<C: OoxmlCodec>(content: ContentData) -> Result<Document> {
    let archive = content.to_bytes();
    let pkg = OoxmlPackage::<C::Format>::open(&archive)?;
    // Cache the property parts' bytes so they can be surfaced as metadata
    // sub-parts without re-opening the package.
    let doc_props = crate::codec::props::read_doc_props(|path| pkg.part_bytes(path));
    let extraction = pkg.extract();

    if !extraction.issues.is_empty() {
        let parts = extraction
            .issues
            .iter()
            .map(|issue| format!("{} ({:?})", issue.part, issue.kind))
            .collect::<Vec<_>>()
            .join(", ");
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

    let state: SharedSplice<OoxmlAddress> = SharedSplice::new(items);
    let body = ExtractStream::new(C::FORMAT_ID, state.clone(), Arc::new(OoxmlAddresser));

    let mut parts: Vec<DocumentPart> = Vec::new();
    parts.push(DocumentPart::Stream {
        id: LocalId::new(BODY_PART_ID),
        handle: ErasedStream::new(C::FORMAT_ID, Box::new(body) as Box<dyn Stream<Text>>),
    });
    // Each binary embedding the engine classifies (images, media, embedded
    // objects, fonts), keyed by its zip entry path and hinted by its extension.
    for embedding in extraction.embeddings {
        let id = LocalId::new(embedding.part.as_str().to_owned());
        let hint = id.extension().unwrap_or_default().to_owned();
        parts.push(DocumentPart::Blob {
            id,
            bytes: embedding.bytes,
            hint,
        });
    }
    // Each document-property part, decoded as the metadata modality.
    for (path, bytes) in doc_props {
        parts.push(DocumentPart::Blob {
            id: LocalId::new(path),
            bytes,
            hint: crate::codec::docprops_hint().to_owned(),
        });
    }

    Ok(Document::new(
        C::FORMAT_ID,
        parts,
        Box::new(OoxmlRecombine::<C> {
            archive,
            state,
            _codec: PhantomData,
        }),
    ))
}

/// A [`DocumentLoader`] for one text-only OOXML format, decoding via
/// [`decode_document`].
#[derive(Debug)]
pub(crate) struct OoxmlLoader<C: OoxmlCodec>(PhantomData<C>);

impl<C: OoxmlCodec> OoxmlLoader<C> {
    /// A loader for codec `C`.
    pub(crate) fn new() -> Self {
        Self(PhantomData)
    }
}

#[async_trait::async_trait]
impl<C: OoxmlCodec> DocumentLoader for OoxmlLoader<C> {
    async fn decode(&self, content: ContentData) -> Result<Document> {
        decode_document::<C>(content)
    }
}

/// Test-only: decode `content` into the body [`ExtractStream`] and its
/// [`OoxmlRecombine`], sharing one item state, so a test can drive the concrete
/// stream (its `TextLocation`-based [`Stream::lift`]) and re-pack through the
/// recombiner, the same pair the [`Document`] holds but without the erasure.
#[cfg(test)]
pub(crate) fn decode_parts<C: OoxmlCodec>(
    content: ContentData,
) -> Result<(ExtractStream<OoxmlAddress>, OoxmlRecombine<C>)> {
    let archive = content.to_bytes();
    let pkg = OoxmlPackage::<C::Format>::open(&archive)?;
    let extraction = pkg.extract();
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
    let state: SharedSplice<OoxmlAddress> = SharedSplice::new(items);
    let stream = ExtractStream::new(C::FORMAT_ID, state.clone(), Arc::new(OoxmlAddresser));
    let recombine = OoxmlRecombine::<C> {
        archive,
        state,
        _codec: PhantomData,
    };
    Ok((stream, recombine))
}
