//! [`Recombine`]: how a format assembles its (possibly redacted) parts back into
//! native bytes, and [`LeafRecombine`], the trivial one-part passthrough.

use elide_core::Result;

use super::EncodedPart;
use crate::content::ContentData;

/// Assemble a [`Document`](super::Document)'s (possibly redacted) parts back into
/// the format's native bytes.
///
/// This is where a format owns recomposition: a DOCX re-packs its zip with the
/// redacted parts, a PDF rebuilds its object graph, an image lays its pixel
/// redactions over its stripped-metadata bytes. A leaf format's recombiner just
/// returns its one part's bytes.
pub trait Recombine: Send + Sync {
    /// Build the document's native bytes from its assembled parts.
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData>;
}

/// The recombiner for a leaf format: a document of one part whose bytes are the
/// document's bytes. A leaf format builds its [`Document`](super::Document) with
/// [`Document::leaf`](super::Document::leaf).
pub struct LeafRecombine;

impl Recombine for LeafRecombine {
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData> {
        match parts {
            [only] => Ok(ContentData::new(only.bytes.clone())),
            _ => Err(elide_core::Error::new(
                elide_core::ErrorKind::Processing,
                "a leaf document must have exactly one part",
            )),
        }
    }
}
