//! Surfacing a PDF's embedded images: each image XObject as the file the
//! image pipeline reads.

use bytes::Bytes;
use elide_core::{Error, Result};
use hayro_syntax::object::{Object, Stream};

use super::{ImageId, SurfacedImage, samples};
use crate::pdf::Pdf;
use crate::pdf::decode::filters;
use crate::pdf::reader::DictExt;

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
    fn of(stream: &Stream<'_>) -> Self {
        // A filter that is not a name or names is no encoding a codec reads.
        let Ok(filters) = filters(stream) else {
            return Self::Unreadable;
        };
        let filters: Vec<&[u8]> = filters
            .iter()
            .map(|filter| filter.name.as_slice())
            .collect();
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
pub(crate) fn is_surfaceable(stream: &Stream<'_>) -> bool {
    match EmbeddingKind::of(stream) {
        EmbeddingKind::Jpeg | EmbeddingKind::Jpeg2000 => true,
        EmbeddingKind::Raw => samples::is_supported(stream.dict()),
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
pub(crate) fn raw_samples(stream: &Stream<'_>) -> Result<Option<Vec<u8>>> {
    match EmbeddingKind::of(stream) {
        EmbeddingKind::Raw => samples::samples(stream).map(Some),
        _ => Ok(None),
    }
}

/// Every image XObject of `doc`, as [`Images`] reads them.
pub(super) fn surfaced(pdf: &Pdf) -> Result<Vec<SurfacedImage>> {
    pdf.file()
        .objects()
        .into_iter()
        .filter_map(|object| match object {
            Object::Stream(stream) if stream.dict().is_subtype(b"Image") => Some(stream),
            _ => None,
        })
        .map(|stream| {
            let object = stream.obj_id();
            let id = ImageId::of(object);
            let (bytes, hint) = match EmbeddingKind::of(&stream) {
                EmbeddingKind::Jpeg => (Bytes::from(stream.raw_data().into_owned()), "jpg"),
                EmbeddingKind::Jpeg2000 => (Bytes::from(stream.raw_data().into_owned()), "jp2"),
                EmbeddingKind::Raw => (Bytes::from(samples::png(&stream)?), "png"),
                EmbeddingKind::Unreadable => {
                    return Err(Error::malformed(format!(
                        "image ({}, {}) cannot be surfaced",
                        object.obj_number, object.gen_number
                    )));
                }
            };
            Ok(SurfacedImage { id, bytes, hint })
        })
        .collect()
}
