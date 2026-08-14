//! [`Embedding`]: a PDF's embedded images, surfaced for extraction and
//! addressed by a typed [`ImageId`].

use bytes::Bytes;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The identity of an image XObject within a PDF: its indirect-object id
/// (object number and generation).
///
/// A newtype so an embedding is never addressed by a bare tuple: extraction
/// hands these back on every [`Embedding`].
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
    pub fn new(number: u32, generation: u16) -> Self {
        Self { number, generation }
    }

    /// The image id for a lopdf `ObjectId` (an `(object number, generation)`
    /// pair).
    pub(crate) fn from_object((number, generation): (u32, u16)) -> Self {
        Self::new(number, generation)
    }

    /// This id as a lopdf `ObjectId` pair.
    #[cfg(feature = "image")]
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

/// One embedded image surfaced for redaction, addressed by its object id.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Embedding {
    /// The image XObject's object id.
    pub id: ImageId,
    /// The 1-based page the image appears on.
    pub page: u32,
    /// How the image is encoded.
    pub kind: EmbeddingKind,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// The image's raw stream bytes (a cheap ref-counted share).
    pub bytes: Bytes,
}
