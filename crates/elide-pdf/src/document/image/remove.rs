//! Remove images from a PDF: cut every drawing of one from the content
//! streams and drop it from the file.

use std::collections::HashSet;

use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::document::content::{apply, drop_xobjects, plan};
use crate::document::object::DocumentExt;

/// Remove the image XObjects `images` from `doc`'s pages and form XObjects,
/// and (when `inline` holds) every inline image, returning how many drawing
/// operations were cut. Each removed XObject is dropped from every resource
/// dictionary too, so the next prune deletes it from the file.
///
/// A page positions everything it draws, so cutting an image leaves its area
/// empty without moving anything else.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a content stream cannot be decoded or
/// encoded.
pub(crate) fn remove(
    doc: &mut Document,
    images: &HashSet<ObjectId>,
    inline: bool,
) -> Result<usize> {
    let edits = plan(doc, |bytes, resources| {
        let content =
            Content::decode(bytes).map_err(|e| Error::malformed(format!("decode content: {e}")))?;
        let before = content.operations.len();
        let kept: Vec<Operation> = content
            .operations
            .into_iter()
            .filter(|operation| match operation.operator.as_str() {
                "BI" => !inline,
                "Do" => !draws(doc, operation, resources, images),
                _ => true,
            })
            .collect();
        let cut = before - kept.len();
        if cut == 0 {
            return Ok(None);
        }
        let encoded = Content { operations: kept }
            .encode()
            .map_err(|e| Error::malformed(format!("encode content: {e}")))?;
        Ok(Some((encoded, cut)))
    })?;
    let cut = apply(doc, edits)?.into_iter().sum();
    drop_xobjects(doc, images);
    Ok(cut)
}

/// Whether the `Do` `operation` draws one of `images`, by its name in
/// `resources`.
fn draws(
    doc: &Document,
    operation: &Operation,
    resources: &[Dictionary],
    images: &HashSet<ObjectId>,
) -> bool {
    let [Object::Name(name)] = operation.operands.as_slice() else {
        return false;
    };
    resources.iter().any(|dict| {
        dict.get(b"XObject")
            .ok()
            .and_then(|entries| doc.resolve_dict(entries))
            .and_then(|entries| entries.get(name).ok())
            .and_then(|xobject| xobject.as_reference().ok())
            .is_some_and(|id| images.contains(&id))
    })
}
