//! A PDF's embedded images: how one is identified and encoded, how it reaches
//! the image pipeline, and how a redacted or unreadable one goes back.
//!
//! An image surfaces as the file the pipeline reads: a JPEG or JPEG 2000 image
//! as it is stored, raw samples decoded to a PNG ([`samples`]). One that cannot
//! surface (fax or JBIG2 samples, samples of an unsupported kind, a stencil
//! mask, an inline image) is removed at decode instead ([`remove`](fn@remove)).
//! A redacted image is written back over its XObject
//! ([`replace`](fn@replace)).

mod remove;
mod replace;
mod samples;

use bytes::Bytes;
use elide_core::{Error, Result};
use lopdf::{Document, Stream};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub(crate) use self::remove::remove;
pub use self::replace::ImageReplacement;
pub(crate) use self::replace::replace;

/// The identity of an image XObject within a PDF: its indirect-object id
/// (object number and generation).
///
/// A newtype so an embedding is never addressed by a bare tuple: extraction
/// hands these back on every [`Embedding`](crate::extract::Embedding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ImageId {
    /// The indirect object number.
    pub number: u32,
    /// The object generation.
    pub generation: u16,
}

impl ImageId {
    /// The image at object `(number, generation)`.
    #[must_use]
    pub fn new(number: u32, generation: u16) -> Self {
        Self { number, generation }
    }

    /// The image id for a lopdf `ObjectId` (an `(object number, generation)`
    /// pair).
    pub(crate) fn from_object((number, generation): (u32, u16)) -> Self {
        Self::new(number, generation)
    }

    /// This id as a lopdf `ObjectId` pair.
    pub(crate) fn object(self) -> (u32, u16) {
        (self.number, self.generation)
    }
}

/// The encoding of an embedded image, from its PDF stream filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum EmbeddingKind {
    /// A JPEG image (`DCTDecode` filter).
    Jpeg,
    /// A JPEG 2000 image (`JPXDecode` filter).
    Jpeg2000,
    /// A CCITT-fax image (`CCITTFaxDecode` filter).
    CcittFax,
    /// A JBIG2 image (`JBIG2Decode` filter).
    Jbig2,
    /// A raw or lossless-filtered image (`FlateDecode`, `LZWDecode`, or none):
    /// the bytes are pixel samples, not a self-contained image file.
    Raw,
}

impl EmbeddingKind {
    /// Classify an image from its PDF stream filter names.
    pub(crate) fn from_filters(filters: Option<&[String]>) -> Self {
        // The last filter in the chain determines the sample encoding.
        match filters.and_then(<[String]>::last).map(String::as_str) {
            Some("DCTDecode") => Self::Jpeg,
            Some("JPXDecode") => Self::Jpeg2000,
            Some("CCITTFaxDecode") => Self::CcittFax,
            Some("JBIG2Decode") => Self::Jbig2,
            _ => Self::Raw,
        }
    }
}

impl EmbeddingKind {
    /// The encoding of the image XObject `stream`, by its filters.
    pub(crate) fn of(stream: &Stream) -> Self {
        let filters: Option<Vec<String>> = stream.filters().ok().map(|filters| {
            filters
                .iter()
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .collect()
        });
        Self::from_filters(filters.as_deref())
    }
}

/// Whether the image XObject `stream` can be surfaced to the image pipeline: a
/// JPEG or JPEG 2000 file as it is, or raw samples the decoder reads.
pub(crate) fn is_surfaceable(doc: &Document, stream: &Stream) -> bool {
    match EmbeddingKind::of(stream) {
        EmbeddingKind::Jpeg | EmbeddingKind::Jpeg2000 => true,
        EmbeddingKind::Raw => samples::is_supported(doc, &stream.dict),
        EmbeddingKind::CcittFax | EmbeddingKind::Jbig2 => false,
    }
}

/// The image `id` of `kind`, stored as `bytes`, as the image pipeline reads it,
/// with its file-extension hint: a JPEG or JPEG 2000 file as it is stored, raw
/// samples decoded to a PNG. `None` for an image of another kind, which the
/// sanitizer removed at decode.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if raw
/// samples the sanitizer kept cannot be decoded.
pub(crate) fn surface(
    doc: &Document,
    id: ImageId,
    kind: EmbeddingKind,
    bytes: Bytes,
) -> Result<Option<(Bytes, &'static str)>> {
    Ok(match kind {
        EmbeddingKind::Jpeg => Some((bytes, "jpg")),
        EmbeddingKind::Jpeg2000 => Some((bytes, "jp2")),
        EmbeddingKind::Raw => {
            let stream = doc
                .get_object(id.object())
                .and_then(lopdf::Object::as_stream)
                .map_err(|e| {
                    Error::malformed(format!("image ({}, {}): {e}", id.number, id.generation))
                })?;
            Some((Bytes::from(samples::png(doc, stream)?), "png"))
        }
        EmbeddingKind::CcittFax | EmbeddingKind::Jbig2 => None,
    })
}
