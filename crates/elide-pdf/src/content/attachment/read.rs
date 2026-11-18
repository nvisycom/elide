//! Reading a PDF's embedded files: the `/EmbeddedFiles` name tree and the
//! `/AF` associated-file arrays, each file decoded.

use std::collections::BTreeSet;

use bytes::Bytes;
use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};

use super::{Attachment, AttachmentId};
use crate::pdf::reader::{DictionaryExt, DocumentExt, NameTree, Node, ObjectExt};

/// The most the attached files may decompress to, together. A stream may
/// claim any size, so each decode is bounded by what is left, to refuse a
/// decompression bomb spread across many files.
const MAX_ATTACHMENT_BYTES: usize = 512 * 1024 * 1024;

/// Every embedded file of `doc`, as [`Attachments`] reads them.
pub(super) fn attachments(doc: &Document) -> Result<Vec<Attachment>> {
    let tree = Tree::read(doc)?;
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    let mut left = MAX_ATTACHMENT_BYTES;
    tree.entries
        .iter()
        .chain(associated(doc).iter())
        .filter(|entry| seen.insert(entry.stream))
        .map(|entry| {
            let stream = doc
                .get_object(entry.stream)
                .and_then(Object::as_stream)
                .map_err(|e| Error::malformed(format!("attachment stream: {e}")))?;
            let over = || {
                Error::resource_limit(format!(
                    "attachment `{}` could not be decoded within the \
                     {MAX_ATTACHMENT_BYTES}-byte limit on all attachments",
                    entry.name
                ))
            };
            let bytes = stream
                .get_plain_content_with_limit(left)
                .map_err(|_| over())?;
            left = left.checked_sub(bytes.len()).ok_or_else(over)?;
            Ok(Attachment {
                id: AttachmentId::from_object(entry.stream),
                name: entry.name.clone(),
                bytes: Bytes::from(bytes),
            })
        })
        .collect()
}

/// The name tree of embedded files under the catalog's `/Names`.
pub(super) const EMBEDDED_FILES: &[u8] = b"EmbeddedFiles";

/// One file in the tree: its name and its embedded-file stream.
#[derive(Debug)]
pub(crate) struct Entry<'a> {
    pub(crate) name: String,
    pub(crate) stream: ObjectId,
    /// The file specification that names it.
    pub(crate) spec: &'a Dictionary,
}

/// The `/EmbeddedFiles` name tree, read once: its nodes (so entries can be
/// dropped in place) and its file entries.
pub(crate) struct Tree<'a> {
    pub(crate) nodes: Vec<Node>,
    pub(crate) entries: Vec<Entry<'a>>,
}

impl<'a> Tree<'a> {
    /// Read the tree of `doc`, empty when the document attaches no file.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// a node of the tree cannot be read, so a file under it would go unseen.
    pub(crate) fn read(doc: &'a Document) -> Result<Self> {
        let tree = NameTree::read(doc, EMBEDDED_FILES);
        if !tree.complete {
            return Err(Error::malformed(
                "attachment name tree cannot be read whole",
            ));
        }
        let entries = tree
            .pairs
            .into_iter()
            .filter_map(|(key, value)| entry(doc, key, value))
            .collect();
        Ok(Self {
            nodes: tree.nodes,
            entries,
        })
    }
}

/// The file entry a name-tree `key` / `value` pair describes, or `None` when the
/// value is not a file specification with an embedded file.
pub(super) fn entry<'a>(doc: &'a Document, key: &Object, value: &'a Object) -> Option<Entry<'a>> {
    let spec = match value {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(dict) => dict,
        _ => return None,
    };
    let files = doc.resolve_dict(spec.get(b"EF").ok()?)?;
    let stream = files
        .get(b"UF")
        .or_else(|_| files.get(b"F"))
        .and_then(Object::as_reference)
        .ok()?;
    let name = spec
        .file_name()
        .or_else(|| decode_text_string(key).ok())
        .unwrap_or_default();
    Some(Entry { name, stream, spec })
}

/// Every file associated through an `/AF` array, on any object of `doc`, in
/// object order.
pub(crate) fn associated(doc: &Document) -> Vec<Entry<'_>> {
    doc.objects
        .values()
        .filter_map(ObjectExt::dict)
        .filter_map(|dict| dict.get(b"AF").ok())
        .filter_map(|af| associations(doc, af))
        .flat_map(|specs| {
            specs
                .iter()
                .filter_map(|spec| entry(doc, &Object::Null, spec))
        })
        .collect()
}

/// The file specifications of an `/AF` value: an inline array, or a reference
/// to one.
pub(super) fn associations<'a>(doc: &'a Document, af: &'a Object) -> Option<&'a Vec<Object>> {
    match af {
        Object::Array(specs) => Some(specs),
        Object::Reference(id) => doc.get_object(*id).and_then(Object::as_array).ok(),
        _ => None,
    }
}
