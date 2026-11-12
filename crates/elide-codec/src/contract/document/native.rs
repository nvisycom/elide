//! [`Native`]: a format's decoded document, which owns its streams and writes
//! itself back into native bytes; [`Encode`] and [`LeafDocument`], the native
//! document of a format that is one self-encoding stream.

use std::marker::PhantomData;

use elide_core::Result;
use elide_core::modality::Modality;

use super::{Blob, LocalId};
use crate::content::ContentData;
use crate::{Stream, StreamMut};

/// A format's decoded document: owns its streams and writes itself back out.
///
/// The engine reaches the streams through [`streams`](Self::streams), each lent
/// type-erased by modality; detection reads them and redaction edits them in
/// place. [`encode`](Self::encode) then reads the same fields, as the concrete
/// types the format built, to write the container: a DOCX re-zips its parts
/// with its body items spliced back, a PDF rebuilds its object graph. Embedded
/// files are not here: they are the [`Document`](super::Document)'s blobs,
/// redacted as documents of their own and handed back to `encode`.
pub trait Native: Send + Sync {
    /// Lend each of the document's streams, with its document-local id, in
    /// document order.
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)>;

    /// Write the document, with its streams' current content and its `blobs`'
    /// current bytes, into native bytes.
    ///
    /// `removed` names the blobs dropped since decode (an embedding no codec
    /// could read); the format removes each from the output along with
    /// everything that references it.
    ///
    /// # Errors
    ///
    /// The document cannot be written, or a removed blob cannot be dropped.
    fn encode(&self, blobs: &[Blob], removed: &[LocalId]) -> Result<ContentData>;

    /// Check the written `output` before it is returned: it holds no hidden
    /// content decode would strip, no structure the format does not recognize,
    /// and no dangling internal reference.
    ///
    /// Required, not defaulted: a format that carries hidden content must say
    /// how its output is checked, and one that carries none says so by
    /// accepting everything.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) if the
    /// output fails a check; the document is refused rather than returned.
    fn validate(&self, output: &ContentData) -> Result<()>;
}

/// A stream that is a whole document by itself, encoding into its bytes.
///
/// A `.txt`, a JSON or CSV file: its redacted content is its output. A leaf
/// [`Loader`](crate::Loader) produces one.
pub trait Encode {
    /// The stream's current content as the document's bytes.
    ///
    /// # Errors
    ///
    /// The in-memory representation cannot be re-encoded.
    fn encode(&self) -> Result<ContentData>;
}

/// The id of a leaf document's one stream.
const LEAF_ID: &str = "body";

/// The native document of a leaf format: one stream that encodes itself.
///
/// Its stream is an `S` of modality `M`. Built by
/// [`Document::leaf`](super::Document::leaf).
pub struct LeafDocument<M, S> {
    stream: S,
    _modality: PhantomData<fn() -> M>,
}

impl<M, S> LeafDocument<M, S> {
    /// The leaf document of `stream`.
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            _modality: PhantomData,
        }
    }
}

impl<M: Modality, S: Stream<M> + Encode> Native for LeafDocument<M, S> {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        vec![(
            LocalId::new(LEAF_ID),
            StreamMut::new::<M, S>(&mut self.stream),
        )]
    }

    fn encode(&self, _blobs: &[Blob], _removed: &[LocalId]) -> Result<ContentData> {
        self.stream.encode()
    }

    /// A leaf format carries no hidden content, so there is nothing to check.
    fn validate(&self, _output: &ContentData) -> Result<()> {
        Ok(())
    }
}
