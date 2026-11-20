//! The shared OOXML codec layer: helpers common to the docx, pptx, and xlsx
//! adapters over this crate's package engine.
//!
//! The two text-only OOXML formats decode to the same
//! [`OoxmlPackage`](crate::ooxml::OoxmlPackage) and build the same
//! [`Document`](elide_codec::Document) the same way; this module holds the pieces
//! that would otherwise be copy-pasted across them: extract element-text blocks
//! addressed by `(part, span, OffsetMap)` into a body
//! [`ExtractStream`](elide_codec::extract::ExtractStream) (the
//! [`addresser`] maps decoded↔raw), surface embeddings as blobs and
//! document-property parts as metadata streams, and re-pack (the native
//! document, [`document`]). The only per-format differences — the engine format,
//! the codec [`FormatId`], the label used in errors, and the extension and MIME
//! type it resolves by — are named by the [`OoxmlCodec`] seam. The sanitize,
//! extraction-issue, blob, and re-pack helpers are shared with the xlsx codec
//! too.

mod addresser;
mod document;
mod loader;

use std::collections::HashSet;
use std::ops::Range;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::{ExtractedItem, OffsetMap, Passage};
use elide_codec::{Blob, Format, FormatId, LocalId, StrippedContent};
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};

pub(crate) use self::addresser::OoxmlAddresser;
#[cfg(test)]
pub(crate) use self::document::OoxmlDocument;
pub(crate) use self::loader::OoxmlLoader;
#[cfg(test)]
pub(crate) use self::loader::decode_native;
use crate::codec::props::DocProps;
use crate::ooxml::OoxmlFormat;
use crate::opc::sanitize::{Sanitizer, remove_parts, sanitize};
use crate::opc::{Block, Embedding, Issue, PartPath, PartReplacement, Replacement};

/// The document-local id of the body stream (not a zip entry), shared by every
/// OOXML document.
pub(crate) const BODY_PART_ID: &str = "body";

/// Ties a text-only OOXML engine format to its codec identity: the engine
/// format it opens, the stable [`FormatId`] it registers under, the short
/// label used in its error messages (`docx`, `pptx`), and the file extension
/// and MIME type it resolves by.
pub(crate) trait OoxmlCodec: Send + Sync + 'static {
    /// The engine format this codec drives.
    type Format: OoxmlFormat;

    /// The stable codec [`FormatId`].
    const FORMAT_ID: FormatId;

    /// A short lowercase name for the format, used in error messages.
    const LABEL: &'static str;

    /// The file extension the format resolves by, without the dot.
    const EXTENSION: &'static str;

    /// The MIME type the format resolves by.
    const CONTENT_TYPE: &'static str;
}

/// The [`Format`] of codec `C`: its loader, resolved by its extension and MIME
/// type.
pub(crate) fn format<C: OoxmlCodec>() -> Format {
    Format::with_document_loader(C::FORMAT_ID, OoxmlLoader::<C>::new())
        .with_extensions([C::EXTENSION])
        .with_content_types([C::CONTENT_TYPE])
}

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

/// The extracted items of an OOXML text stream: one per block, its value the
/// block's decoded text, addressed by its part, raw span, and offset map.
pub(crate) fn items(blocks: Vec<Block>) -> Vec<ExtractedItem<OoxmlAddress>> {
    blocks
        .into_iter()
        .map(|block| ExtractedItem {
            value: block.text.to_string(),
            address: OoxmlAddress {
                part: block.part,
                span: block.start..block.end,
                offsets: block.offsets,
            },
            hints: Vec::new(),
            passage: block.passage,
            flag: block.flag,
        })
        .collect()
}

/// `content` sanitized under `policy` by `sanitizer`: the package bytes that
/// will ship (the source itself when nothing was stripped), and what was
/// stripped.
///
/// # Errors
///
/// [`MalformedInput`](elide_core::ErrorKind::MalformedInput) if the bytes are
/// not a package this crate can sanitize.
pub(crate) fn sanitized(
    content: &ContentData,
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<(Bytes, StrippedContent)> {
    let source = content.to_bytes();
    let (sanitized, stripped) = sanitize(&source, policy, sanitizer)?;
    Ok((sanitized.map_or(source, Bytes::from), stripped))
}

/// Refuse a decode whose extraction left `issues`: a text-bearing part that
/// could not be read would ship its text un-redacted. `label` names the format
/// in the error.
///
/// # Errors
///
/// [`MalformedInput`](elide_core::ErrorKind::MalformedInput) naming every
/// unread part, if `issues` is not empty.
pub(crate) fn refuse_issues(label: &str, issues: &[Issue]) -> Result<()> {
    if issues.is_empty() {
        return Ok(());
    }
    let parts = issues
        .iter()
        .map(|issue| format!("{} ({:?})", issue.part, issue.kind))
        .collect::<Vec<_>>()
        .join(", ");
    Err(Error::malformed(format!(
        "{label}: text-bearing part(s) could not be extracted: {parts}"
    )))
}

/// One blob per binary embedding (images, media, embedded objects), keyed by
/// its zip entry path and hinted by its extension.
pub(crate) fn blobs(embeddings: Vec<Embedding>) -> Vec<Blob> {
    embeddings
        .into_iter()
        .map(|embedding| {
            let id = LocalId::new(embedding.part.as_str().to_owned());
            let hint = id.extension().unwrap_or_default().to_owned();
            Blob {
                id,
                bytes: embedding.bytes,
                hint,
            }
        })
        .collect()
}

/// The whole-part replacements of an encode: each property part's (possibly
/// redacted) XML and each blob's current bytes, written over the package part
/// its id names. An unredacted one's bytes equal the original, so its
/// replacement is a no-op re-pack.
pub(crate) fn part_replacements(properties: &DocProps, blobs: &[Blob]) -> Vec<PartReplacement> {
    let blobs = blobs
        .iter()
        .map(|blob| PartReplacement::new(PartPath::from(blob.id.as_str()), blob.bytes.clone()));
    properties.replacements().into_iter().chain(blobs).collect()
}

/// The text replacements that write each item's current value back over its
/// raw span; a separator is never written back.
pub(crate) fn replacements(items: &[ExtractedItem<OoxmlAddress>]) -> Vec<Replacement> {
    items
        .iter()
        .filter(|item| item.passage != Passage::Separator)
        .map(|item| Replacement {
            part: item.address.part.clone(),
            start: item.address.span.start,
            end: item.address.span.end,
            text: item.value.clone().into(),
        })
        .collect()
}

/// The re-packed `package` as encoded content, without the blobs `removed`
/// names (each id is its zip entry path), along with every relationship and
/// element that references one.
///
/// # Errors
///
/// [`MalformedInput`](elide_core::ErrorKind::MalformedInput) if the package
/// cannot be read or repacked.
pub(crate) fn finish(package: Vec<u8>, removed: &[LocalId]) -> Result<ContentData> {
    let gone: HashSet<PartPath> = removed
        .iter()
        .map(|id| PartPath::from(id.as_str()))
        .collect();
    let package = remove_parts(&package, gone)?.unwrap_or(package);
    Ok(ContentData::new(Bytes::from(package)))
}
