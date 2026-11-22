//! Redacting a PDF's embedded files: a redacted file written back into its
//! stream, and every reference to a file no codec could read dropped.

use std::collections::HashSet;

use elide_core::{Error, Result};
use lopdf::{Object, ObjectId};

use super::read::{EMBEDDED_FILES, Tree, associated, associations, entry};
use super::{AttachmentId, AttachmentReplacement};
use crate::pdf::Edit;
use crate::pdf::reader::{Node, ObjectExt};

/// Apply an [`AttachmentRedaction`] to `edit`.
pub(super) fn replace(
    edit: &mut Edit,
    replacements: &[AttachmentReplacement],
    removed: &[AttachmentId],
) -> Result<()> {
    let doc = edit.doc();
    let Tree { nodes, entries, .. } = Tree::read(doc)?;
    let known: HashSet<ObjectId> = entries
        .iter()
        .chain(associated(doc).iter())
        .map(|entry| entry.stream)
        .collect();
    let unknown = replacements
        .iter()
        .map(|r| r.id)
        .chain(removed.iter().copied())
        .find(|id| !known.contains(&id.object()));
    if let Some(id) = unknown {
        return Err(Error::redaction(format!(
            "object ({}, {}) is not an attachment of this document",
            id.number, id.generation
        )));
    }

    for replacement in replacements {
        write_back(edit, replacement)?;
    }

    let removed: HashSet<ObjectId> = removed.iter().map(|id| id.object()).collect();
    if !removed.is_empty() {
        drop_entries(edit, &nodes, &removed);
        drop_associated(edit, &removed);
    }
    Ok(())
}

/// Write `replacement`'s bytes into its embedded-file stream in `edit`,
/// compressed where they compress, and its `/Params` `/Size` to their length.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// the attachment is not a stream.
fn write_back(edit: &mut Edit, replacement: &AttachmentReplacement) -> Result<()> {
    let id = replacement.id;
    let Ok(Object::Stream(stream)) = edit.doc_mut().get_object_mut(id.object()) else {
        return Err(Error::malformed(format!(
            "attachment ({}, {}) is not a stream",
            id.number, id.generation
        )));
    };
    stream.set_plain_content(replacement.bytes.clone());
    // Best effort: an uncompressible file is stored plain.
    let _ = stream.compress();
    let size = Object::Integer(replacement.bytes.len() as i64);
    let params = match stream.dict.get_mut(b"Params") {
        Ok(Object::Dictionary(params)) => Some(params),
        Ok(Object::Reference(params)) => {
            let params = *params;
            edit.doc_mut()
                .get_object_mut(params)
                .ok()
                .and_then(|params| params.as_dict_mut().ok())
        }
        _ => None,
    };
    if let Some(params) = params {
        params.set("Size", size);
    }
    Ok(())
}

/// Drop every entry whose embedded-file stream is in `removed` from the tree
/// `nodes` of `edit`.
fn drop_entries(edit: &mut Edit, nodes: &[Node], removed: &HashSet<ObjectId>) {
    edit.retain_names(EMBEDDED_FILES, nodes, |doc, key, value| {
        entry(doc, key, value).is_none_or(|entry| !removed.contains(&entry.stream))
    });
}

/// Drop every `/AF` association whose file is in `removed`, removing an array
/// left empty.
fn drop_associated(edit: &mut Edit, removed: &HashSet<ObjectId>) {
    let doc = edit.doc_mut();
    // Decide which arrays change while the document is readable: an
    // association resolves to its stream through a (possibly referenced) file
    // specification, which a mutable borrow would block.
    let mut edits: Vec<(ObjectId, Option<ObjectId>, Vec<Object>)> = Vec::new();
    for (&owner, object) in &doc.objects {
        let Some(af) = object.dict().and_then(|dict| dict.get(b"AF").ok()) else {
            continue;
        };
        let Some(specs) = associations(doc, af) else {
            continue;
        };
        let kept: Vec<Object> = specs
            .iter()
            .filter(|spec| {
                entry(doc, &Object::Null, spec).is_none_or(|e| !removed.contains(&e.stream))
            })
            .cloned()
            .collect();
        if kept.len() != specs.len() {
            edits.push((owner, af.as_reference().ok(), kept));
        }
    }
    for (owner, array, kept) in edits {
        match array {
            // The array is its own object: rewrite it, and drop it from its
            // owner once empty.
            Some(array) => {
                let empty = kept.is_empty();
                doc.objects.insert(array, Object::Array(kept));
                if empty
                    && let Some(dict) = doc.get_object_mut(owner).ok().and_then(ObjectExt::dict_mut)
                {
                    dict.remove(b"AF");
                }
            }
            None => {
                if let Some(dict) = doc.get_object_mut(owner).ok().and_then(ObjectExt::dict_mut) {
                    if kept.is_empty() {
                        dict.remove(b"AF");
                    } else {
                        dict.set("AF", Object::Array(kept));
                    }
                }
            }
        }
    }
}
