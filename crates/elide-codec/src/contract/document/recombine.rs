//! [`Recombine`]: how a format assembles its (possibly redacted) parts back into
//! native bytes, and [`LeafRecombine`], the trivial one-part passthrough.

use elide_core::{Error, ErrorKind, Result};

use super::{EncodedPart, LocalId};
use crate::content::ContentData;

/// Assemble a [`Document`](super::Document)'s (possibly redacted) parts back into
/// the format's native bytes, then check the result.
///
/// This is where a format owns recomposition: a DOCX re-packs its zip with the
/// redacted parts, a PDF rebuilds its object graph, an image lays its pixel
/// redactions over its stripped-metadata bytes. A leaf format's recombiner just
/// returns its one part's bytes.
pub trait Recombine: Send + Sync {
    /// Build the document's native bytes from its assembled parts.
    ///
    /// `removed` names the blob parts dropped since decode (an embedding no
    /// codec could read); the format removes each from the output along with
    /// everything that references it.
    ///
    /// # Errors
    ///
    /// The parts cannot be assembled, or a removed part cannot be dropped.
    fn assemble(&self, parts: &[EncodedPart], removed: &[LocalId]) -> Result<ContentData>;

    /// Check the assembled `output` before it is returned: it holds no hidden
    /// content decode would strip, no structure the format does not recognize,
    /// and no dangling internal reference.
    ///
    /// Required, not defaulted: a format that carries hidden content must say
    /// how its output is checked, and one that carries none says so by
    /// accepting everything.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`] if the output fails a check; the document is
    /// refused rather than returned.
    fn validate(&self, output: &ContentData) -> Result<()>;
}

/// The recombiner for a leaf format: a document of one part whose bytes are the
/// document's bytes.
///
/// A leaf format builds its [`Document`](super::Document) with
/// [`Document::leaf`](super::Document::leaf).
pub struct LeafRecombine;

impl Recombine for LeafRecombine {
    fn assemble(&self, parts: &[EncodedPart], _removed: &[LocalId]) -> Result<ContentData> {
        match parts {
            [only] => Ok(ContentData::new(only.bytes.clone())),
            _ => Err(Error::new(
                ErrorKind::Processing,
                "a leaf document must have exactly one part",
            )),
        }
    }

    /// A leaf format carries no hidden content, so there is nothing to check.
    fn validate(&self, _output: &ContentData) -> Result<()> {
        Ok(())
    }
}
