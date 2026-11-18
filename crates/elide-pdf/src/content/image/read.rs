//! Surfacing a PDF's embedded images: each image XObject as the file the
//! image pipeline reads.

use bytes::Bytes;
use elide_core::{Error, Result};
use lopdf::{Document, Object, Stream};

use super::{ImageId, SurfacedImage, samples};
use crate::pdf::reader::DictionaryExt;

/// The encoding of an embedded image, from its PDF stream filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddingKind {
    /// A JPEG image (`DCTDecode` filter).
    Jpeg,
    /// A JPEG 2000 image (`JPXDecode` filter).
    Jpeg2000,
    /// A CCITT-fax or JBIG2 image, or a JPEG under a further filter, which no
    /// image codec reads as stored.
    Unreadable,
    /// A raw or lossless-filtered image (`FlateDecode`, `LZWDecode`, or none):
    /// the bytes are pixel samples, not a self-contained image file.
    Raw,
}

impl EmbeddingKind {
    /// The encoding of the image XObject `stream`: the last filter in its chain
    /// determines the sample encoding.
    fn of(stream: &Stream) -> Self {
        let filters = stream.filters().unwrap_or_default();
        match filters.as_slice() {
            [b"DCTDecode"] => Self::Jpeg,
            [b"JPXDecode"] => Self::Jpeg2000,
            [
                ..,
                b"DCTDecode" | b"JPXDecode" | b"CCITTFaxDecode" | b"JBIG2Decode",
            ] => Self::Unreadable,
            _ => Self::Raw,
        }
    }
}

/// Whether the image XObject `stream` can be surfaced to the image pipeline: a
/// JPEG or JPEG 2000 file as it is, or raw samples the decoder reads.
pub(crate) fn is_surfaceable(doc: &Document, stream: &Stream) -> bool {
    match EmbeddingKind::of(stream) {
        EmbeddingKind::Jpeg | EmbeddingKind::Jpeg2000 => true,
        EmbeddingKind::Raw => samples::is_supported(doc, &stream.dict),
        EmbeddingKind::Unreadable => false,
    }
}

/// The samples of the image XObject `stream` in `doc` when it stores raw
/// samples, exactly as many as the image takes (see [`samples::samples`]);
/// `None` for an image stored as a file.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// its samples are of an unsupported kind or cannot be decompressed.
pub(crate) fn raw_samples(doc: &Document, stream: &Stream) -> Result<Option<Vec<u8>>> {
    match EmbeddingKind::of(stream) {
        EmbeddingKind::Raw => samples::samples(doc, stream).map(Some),
        _ => Ok(None),
    }
}

/// Every image XObject of `doc`, as [`Images`] reads them.
pub(super) fn surfaced(doc: &Document) -> Result<Vec<SurfacedImage>> {
    doc.objects
        .iter()
        .filter_map(|(&id, object)| match object {
            Object::Stream(stream) if stream.dict.is_subtype(doc, b"Image") => Some((id, stream)),
            _ => None,
        })
        .map(|((number, generation), stream)| {
            let id = ImageId::new(number, generation);
            let (bytes, hint) = match EmbeddingKind::of(stream) {
                EmbeddingKind::Jpeg => (Bytes::copy_from_slice(&stream.content), "jpg"),
                EmbeddingKind::Jpeg2000 => (Bytes::copy_from_slice(&stream.content), "jp2"),
                EmbeddingKind::Raw => (Bytes::from(samples::png(doc, stream)?), "png"),
                EmbeddingKind::Unreadable => {
                    return Err(Error::malformed(format!(
                        "image ({number}, {generation}) cannot be surfaced"
                    )));
                }
            };
            Ok(SurfacedImage { id, bytes, hint })
        })
        .collect()
}
