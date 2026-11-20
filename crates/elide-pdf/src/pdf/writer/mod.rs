//! Changing a PDF: an [`Edit`] of one copy of a document, finished into a
//! new [`Pdf`].

mod cmap;
mod content;
mod name_tree;

use std::collections::HashSet;

use bytes::Bytes;
use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

pub(crate) use self::cmap::{UnicodeMap, scrub};
use super::reader::{Node, ObjectExt, Scope};
use super::{Pdf, Redact};

/// A stream of `dict` holding `content`, compressed.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// `content` cannot be compressed.
pub(crate) fn compressed(dict: Dictionary, content: Vec<u8>) -> Result<Stream> {
    let mut stream = Stream::new(dict, content);
    stream
        .compress()
        .map_err(|e| Error::malformed(format!("could not compress a stream: {e}")))?;
    Ok(stream)
}

/// One copy of a document being changed: redactions are written into it, and
/// [`finish`](Edit::finish) saves it once as a new [`Pdf`].
#[derive(Debug, Clone)]
pub(crate) struct Edit {
    doc: Document,
}

impl Edit {
    /// An edit of `doc`.
    pub(crate) fn of(doc: Document) -> Self {
        Self { doc }
    }

    /// The document as edited so far.
    pub(crate) fn doc(&self) -> &Document {
        &self.doc
    }

    /// The document, to change.
    pub(crate) fn doc_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    /// Write `redaction` into the document.
    ///
    /// # Errors
    ///
    /// As `redaction`.
    pub(crate) fn redact<R: Redact>(&mut self, redaction: R) -> Result<()> {
        redaction.redact(self)
    }

    /// Rewrite every content stream `rewrite` changes: each page's (each set
    /// of shared streams once), and each form XObject's, tiling pattern's and
    /// Type3 glyph procedure's, planned against the document before any is
    /// written. `rewrite` reads the document, a stream's bytes and the scope
    /// its names resolve in, and returns its new bytes and what it reports
    /// cutting; what each changed stream reports is returned.
    ///
    /// # Errors
    ///
    /// As `rewrite`, and
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// a stream cannot be read or written back.
    pub(crate) fn rewrite_contents<C>(
        &mut self,
        rewrite: impl FnMut(&Document, &[u8], &Scope<'_>) -> Result<Option<(Vec<u8>, C)>>,
    ) -> Result<Vec<C>> {
        let edits = content::plan(&self.doc, rewrite)?;
        content::apply(&mut self.doc, edits)
    }

    /// Drop every entry naming one of `xobjects` from every resource
    /// dictionary's `/XObject`, so the next save prunes what only they
    /// reached.
    pub(crate) fn drop_xobjects(&mut self, xobjects: &HashSet<ObjectId>) {
        content::drop_xobjects(&mut self.doc, xobjects);
    }

    /// Drop from the name tree `tree`, at each of its `nodes`, every pair
    /// `keep` rejects.
    pub(crate) fn retain_names(
        &mut self,
        tree: &[u8],
        nodes: &[Node],
        keep: impl Fn(&Document, &Object, &Object) -> bool,
    ) {
        name_tree::retain(&mut self.doc, tree, nodes, keep);
    }

    /// The document saved as a new [`Pdf`]: every object it no longer
    /// reaches is dropped first (a superseded content stream, a replaced
    /// image's mask), since lopdf saves an unreachable object too.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the document cannot be saved;
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// it saves past [`Pdf::MAX_DOCUMENT_BYTES`].
    pub(crate) fn finish(self) -> Result<Pdf> {
        let mut doc = self.doc;
        doc.objects.retain(|_, object| !object.is_xref_stream());
        doc.prune_objects();
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes)
            .map_err(|e| Error::malformed(format!("could not save PDF: {e}")))?;
        // What is written is opened again (to check it, or to redact it), so
        // it is held to the limit any document is opened within.
        if bytes.len() > Pdf::MAX_DOCUMENT_BYTES {
            return Err(Error::resource_limit(format!(
                "the document written is {} bytes, over the {}-byte limit",
                bytes.len(),
                Pdf::MAX_DOCUMENT_BYTES
            )));
        }
        Ok(Pdf::from_parts(doc, Bytes::from(bytes)))
    }
}
