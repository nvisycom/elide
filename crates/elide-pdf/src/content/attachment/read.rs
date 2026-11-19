//! Reading a PDF's embedded files: the `/EmbeddedFiles` name tree's, then
//! each associated through an `/AF` array.

use std::collections::BTreeSet;

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use hayro_syntax::object::{Array, Dict, Object};

use super::{Attachment, AttachmentId};
use crate::pdf::Pdf;
use crate::pdf::reader::{DictExt, NameTree, Node, ObjectId, Pair, StreamExt, text_string};

/// The most the attached files may decompress to, together. A stream may
/// claim any size, so each decode is bounded by what is left, to refuse a
/// decompression bomb spread across many files.
const MAX_ATTACHMENT_BYTES: usize = 512 * 1024 * 1024;

/// Every embedded file of `pdf`, as [`Attachments`](super::Attachments)
/// reads them.
pub(super) fn attachments(pdf: &Pdf) -> Result<Vec<Attachment>> {
    let tree = Tree::read(pdf)?;
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    let mut left = MAX_ATTACHMENT_BYTES;
    tree.entries
        .iter()
        .chain(associated(pdf).iter())
        .filter(|entry| seen.insert(entry.stream))
        .map(|entry| {
            let stream = pdf
                .object(entry.stream)
                .and_then(Object::into_stream)
                .ok_or_else(|| Error::malformed("an attachment's file is not a stream"))?;
            let over = || {
                Error::resource_limit(format!(
                    "attachment `{}` could not be decoded within the \
                     {MAX_ATTACHMENT_BYTES}-byte limit on all attachments",
                    entry.name
                ))
            };
            let bytes = stream.plain_within(left).map_err(|e| match e.kind() {
                ErrorKind::ResourceLimit => over(),
                _ => Error::malformed(format!(
                    "attachment `{}` cannot be decoded: {e}",
                    entry.name
                )),
            })?;
            left = left.checked_sub(bytes.len()).ok_or_else(over)?;
            Ok(Attachment {
                id: AttachmentId::of(entry.stream),
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
    /// The tree's value for it: the file specification.
    pub(crate) spec: Dict<'a>,
    /// The file specification's object, when it is one of its own.
    pub(crate) spec_id: Option<ObjectId>,
}

/// The `/EmbeddedFiles` name tree, read once: its nodes (so entries can be
/// dropped in place) and its file entries.
pub(crate) struct Tree<'a> {
    pub(crate) nodes: Vec<Node>,
    pub(crate) entries: Vec<Entry<'a>>,
}

impl<'a> Tree<'a> {
    /// Read the tree of `pdf`, empty when the document attaches no file.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// a node of the tree cannot be read, so a file under it would go unseen.
    pub(crate) fn read(pdf: &'a Pdf) -> Result<Self> {
        let tree = NameTree::read(pdf, EMBEDDED_FILES);
        if !tree.complete {
            return Err(Error::malformed(
                "attachment name tree cannot be read whole",
            ));
        }
        let entries = tree.pairs.iter().filter_map(entry).collect();
        Ok(Self {
            nodes: tree.nodes,
            entries,
        })
    }
}

/// The file entry a name-tree pair describes, or `None` when the value is
/// not a file specification with an embedded file.
pub(super) fn entry<'a>(pair: &Pair<'a>) -> Option<Entry<'a>> {
    let key = match &pair.key {
        Object::String(key) => Some(text_string(key.as_bytes())),
        _ => None,
    };
    spec_entry(pair.value.clone().into_dict()?, pair.id, key)
}

/// The file entry the file specification `spec` (the object `spec_id`, when
/// it is one), named `key` where it names nothing, describes.
fn spec_entry<'a>(
    spec: Dict<'a>,
    spec_id: Option<ObjectId>,
    key: Option<String>,
) -> Option<Entry<'a>> {
    let (stream, name) = spec.embedded_file()?;
    Some(Entry {
        name: name.or(key).unwrap_or_default(),
        stream: stream.obj_id(),
        spec,
        spec_id,
    })
}

/// Every file associated through an `/AF` array, on any object of `pdf`, in
/// object order.
pub(crate) fn associated(pdf: &Pdf) -> Vec<Entry<'_>> {
    pdf.file()
        .objects()
        .into_iter()
        .filter_map(|object| match object {
            Object::Dict(dict) => Some(dict),
            Object::Stream(stream) => Some(stream.dict().clone()),
            _ => None,
        })
        .filter_map(|dict| dict.get::<Array<'_>>(b"AF"))
        .flat_map(|specs| associations(pdf, &specs))
        .collect()
}

/// The file entries of the `/AF` array `specs`, each a file specification
/// held in place or referred to.
pub(super) fn associations<'a>(pdf: &'a Pdf, specs: &Array<'a>) -> Vec<Entry<'a>> {
    specs
        .raw_iter()
        .filter_map(|spec| {
            let id = spec.as_obj_ref().map(ObjectId::from);
            spec_entry(pdf.resolve(&spec)?.into_dict()?, id, None)
        })
        .collect()
}
