//! [`OoxmlLoader`]: build a [`Document`] from a text-only OOXML package.

use std::marker::PhantomData;

use elide_codec::content::ContentData;
use elide_codec::extract::ExtractStream;
use elide_codec::{Blob, Document, DocumentLoader, StrippedContent};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::addresser::OoxmlAddresser;
use super::document::OoxmlDocument;
use super::{OoxmlCodec, blobs, items, refuse_issues, sanitized};
use crate::codec::props::DocProps;
use crate::ooxml::{OoxmlFormat, OoxmlPackage};

/// A decoded OOXML package: its native document, its embedded blobs, and what
/// decode stripped.
struct Decoded<C: OoxmlCodec> {
    native: OoxmlDocument<C>,
    blobs: Vec<Blob>,
    stripped: StrippedContent,
}

/// Sanitize the package under `policy`, extract its text blocks and
/// embeddings, and build its native document and blobs.
///
/// **Fail-closed:** a non-empty [`issues`](crate::opc::Extraction::issues) list
/// means a text-bearing part could not be read into blocks, so its text would
/// ship un-redacted; that refuses the decode rather than emitting a
/// partially-extracted document.
///
/// Only embeddings the engine classifies as
/// [`Binary`](crate::opc::PartRole::Binary) are surfaced as blobs, and the
/// document-property parts as metadata streams when metadata is treated as
/// content; a text or structure part is never exposed, so it can't be
/// clobbered past the text-splice path.
fn decode_package<C: OoxmlCodec>(
    content: ContentData,
    policy: HiddenContentPolicy,
) -> Result<Decoded<C>> {
    let (archive, stripped) = sanitized(&content, policy, &C::Format::classifier())?;
    let pkg = OoxmlPackage::<C::Format>::open(&archive)?;
    let properties = DocProps::under(policy, |path| pkg.part_bytes(path));
    let extraction = pkg.extract();
    refuse_issues(C::EXTENSION, &extraction.issues)?;

    let body = ExtractStream::new(C::FORMAT_ID, items(extraction.blocks), OoxmlAddresser);
    let blobs = blobs(extraction.embeddings);

    Ok(Decoded {
        native: OoxmlDocument {
            body,
            properties,
            archive,
            policy,
            _codec: PhantomData,
        },
        blobs,
        stripped,
    })
}

/// A [`DocumentLoader`] for one text-only OOXML format. Shared by the DOCX and
/// PPTX codecs.
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
    /// Decode `content` into a [`Document`]: an [`OoxmlDocument`] (a body
    /// [`ExtractStream`] and a metadata stream per document-property part)
    /// plus a [`Blob`] per surfaced embedding.
    ///
    /// # Errors
    ///
    /// - [`MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
    ///   bytes are not a valid package of the format, hold a part this crate
    ///   cannot redact, or a text-bearing part could not be extracted.
    async fn decode(&self, content: ContentData, policy: HiddenContentPolicy) -> Result<Document> {
        let decoded = decode_package::<C>(content, policy)?;
        Ok(Document::new(C::FORMAT_ID, decoded.native, decoded.blobs)
            .with_stripped(decoded.stripped))
    }
}

/// Test-only: decode `content` under `policy` into its [`OoxmlDocument`], so a
/// test can drive the concrete body stream (its `TextLocation`-based
/// `Stream::lift`) and write the document without the output check.
#[cfg(test)]
pub(crate) fn decode_native<C: OoxmlCodec>(
    content: ContentData,
    policy: HiddenContentPolicy,
) -> Result<OoxmlDocument<C>> {
    Ok(decode_package::<C>(content, policy)?.native)
}
