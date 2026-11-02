//! [`OoxmlLoader`] + [`decode_document`]: build a parts-model [`Document`] from a
//! text-only OOXML package.

use std::iter;
use std::marker::PhantomData;
use std::sync::Arc;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::{ExtractStream, SharedSplice};
use elide_codec::{
    Document, DocumentLoader, DocumentPart, ErasedStream, LocalId, Stream, StrippedContent,
};
use elide_core::modality::text::Text;
use elide_core::redaction::{Disposition, HiddenContentPolicy};
use elide_core::{Error, Result};

use super::addresser::OoxmlAddresser;
use super::recombine::OoxmlRecombine;
use super::{BODY_PART_ID, OoxmlAddress, OoxmlCodec, items};
use crate::codec::docprops_hint;
use crate::codec::props::read_doc_props;
use crate::ooxml::{OoxmlFormat, OoxmlPackage};
use crate::opc::sanitize::sanitize;

/// The body of a decoded OOXML package: its text stream, the recombiner sharing
/// its item state, and the blob parts surfaced beside it.
struct Decoded<C: OoxmlCodec> {
    stream: ExtractStream<OoxmlAddress>,
    recombine: OoxmlRecombine<C>,
    blobs: Vec<DocumentPart>,
    stripped: StrippedContent,
}

/// Sanitize the package under `policy`, extract its text blocks and
/// embeddings, and build its body stream, recombiner, and blob parts.
///
/// **Fail-closed:** a non-empty [`issues`](crate::opc::Extraction::issues) list
/// means a text-bearing part could not be read into blocks, so its text would
/// ship un-redacted; that refuses the decode rather than emitting a
/// partially-extracted document.
///
/// Only embeddings the engine classifies as
/// [`Binary`](crate::opc::PartRole::Binary), and the document-property parts
/// when metadata is redacted, are surfaced as blobs; a text or structure part
/// is never exposed, so it can't be clobbered past the text-splice path.
fn decode_package<C: OoxmlCodec>(
    content: ContentData,
    policy: HiddenContentPolicy,
) -> Result<Decoded<C>> {
    let source = content.to_bytes();
    let (sanitized, stripped) = sanitize(&source, policy, &C::Format::classifier())?;
    let archive = sanitized.map_or(source, Bytes::from);
    let pkg = OoxmlPackage::<C::Format>::open(&archive)?;
    let doc_props = match policy.metadata {
        Disposition::Redact => read_doc_props(|path| pkg.part_bytes(path)),
        Disposition::Strip => Vec::new(),
    };
    let extraction = pkg.extract();

    if !extraction.issues.is_empty() {
        let parts = extraction
            .issues
            .iter()
            .map(|issue| format!("{} ({:?})", issue.part, issue.kind))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::malformed(format!(
            "{}: text-bearing part(s) could not be extracted: {parts}",
            C::LABEL
        )));
    }

    let state: SharedSplice<OoxmlAddress> = SharedSplice::new(items(extraction.blocks));
    let stream = ExtractStream::new(C::FORMAT_ID, state.clone(), Arc::new(OoxmlAddresser));

    // Each binary embedding the engine classifies (images, media, embedded
    // objects), keyed by its zip entry path and hinted by its extension; then
    // each document-property part, decoded as the metadata modality.
    let embeddings = extraction.embeddings.into_iter().map(|embedding| {
        let id = LocalId::new(embedding.part.as_str().to_owned());
        let hint = id.extension().unwrap_or_default().to_owned();
        DocumentPart::Blob {
            id,
            bytes: embedding.bytes,
            hint,
        }
    });
    let properties = doc_props
        .into_iter()
        .map(|(path, bytes)| DocumentPart::Blob {
            id: LocalId::new(path),
            bytes,
            hint: docprops_hint().to_owned(),
        });

    Ok(Decoded {
        stream,
        recombine: OoxmlRecombine {
            archive,
            state,
            policy,
            _codec: PhantomData,
        },
        blobs: embeddings.chain(properties).collect(),
        stripped,
    })
}

/// Decode `content` into the parts-model [`Document`]: a body
/// [`ExtractStream`] plus a [`Blob`](DocumentPart::Blob) per surfaced embedding
/// and document-property part. Shared by the DOCX and PPTX loaders.
///
/// # Errors
///
/// - [`MalformedInput`](elide_core::ErrorKind::MalformedInput) if the bytes are not a valid
///   package of the format, hold a part this crate cannot redact, or a
///   text-bearing part could not be extracted.
pub(crate) fn decode_document<C: OoxmlCodec>(
    content: ContentData,
    policy: HiddenContentPolicy,
) -> Result<Document> {
    let decoded = decode_package::<C>(content, policy)?;
    let body = DocumentPart::Stream {
        id: LocalId::new(BODY_PART_ID),
        handle: ErasedStream::new(
            C::FORMAT_ID,
            Box::new(decoded.stream) as Box<dyn Stream<Text>>,
        ),
    };
    let parts = iter::once(body).chain(decoded.blobs).collect();
    Ok(
        Document::new(C::FORMAT_ID, parts, Box::new(decoded.recombine))
            .with_stripped(decoded.stripped),
    )
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
    async fn decode(&self, content: ContentData, policy: HiddenContentPolicy) -> Result<Document> {
        decode_document::<C>(content, policy)
    }
}

/// Test-only: decode `content` under `policy` into the body [`ExtractStream`]
/// and its [`OoxmlRecombine`], sharing one item state, so a test can drive the
/// concrete stream (its `TextLocation`-based [`Stream::lift`]) and re-pack
/// through the recombiner, the same pair the [`Document`] holds but without the
/// erasure.
#[cfg(test)]
pub(crate) fn decode_parts<C: OoxmlCodec>(
    content: ContentData,
    policy: HiddenContentPolicy,
) -> Result<(ExtractStream<OoxmlAddress>, OoxmlRecombine<C>)> {
    let decoded = decode_package::<C>(content, policy)?;
    Ok((decoded.stream, decoded.recombine))
}
