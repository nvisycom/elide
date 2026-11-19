//! Redacting a PDF's embedded files: a redacted file written back into its
//! stream, and every reference to a file no codec could read dropped.

use std::collections::BTreeSet;

use elide_core::{Error, Result};
use hayro_syntax::object::{Array, MaybeRef, Object};

use super::read::{EMBEDDED_FILES, Tree, associated, entry};
use super::{AttachmentId, AttachmentReplacement};
use crate::pdf::object::Value;
use crate::pdf::reader::{DictExt, ObjectId};
use crate::pdf::writer::{Changes, compressed, dict_value, retain_names, stream_value, value};
use crate::pdf::{Edit, Pdf};

/// Apply an [`AttachmentRedaction`](super::AttachmentRedaction) to `edit`.
pub(super) fn replace(
    edit: &mut Edit,
    replacements: &[AttachmentReplacement],
    removed: &[AttachmentId],
) -> Result<()> {
    let pdf = edit.pdf();
    let Tree { nodes, entries } = Tree::read(pdf)?;
    let known: BTreeSet<ObjectId> = entries
        .iter()
        .chain(associated(pdf).iter())
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

    let mut changes = Changes::new();
    for replacement in replacements {
        let Some(stream) = pdf
            .object(replacement.id.object())
            .and_then(Object::into_stream)
        else {
            return Err(Error::malformed(format!(
                "attachment ({}, {}) is not a stream",
                replacement.id.number, replacement.id.generation
            )));
        };
        let mut dict = stream_value(&stream).dict;
        if let Some(params) = dict.get_mut(b"Params").and_then(Value::as_dict_mut) {
            params.set("Size", replacement.bytes.len());
        }
        changes.replace(
            replacement.id.object(),
            compressed(dict, &replacement.bytes)?,
        );
    }

    let removed: BTreeSet<ObjectId> = removed.iter().map(|id| id.object()).collect();
    if !removed.is_empty() {
        retain_names(
            pdf,
            EMBEDDED_FILES,
            &nodes,
            |pair| entry(pair).is_none_or(|entry| !removed.contains(&entry.stream)),
            &mut changes,
        );
        drop_associated(pdf, &removed, &mut changes);
    }
    edit.apply(changes)
}

/// Drop in `changes` every `/AF` association of `pdf` naming one of the
/// `removed` files: from the array, or the array whole, with its `/AF`
/// entry, once it names none.
fn drop_associated(pdf: &Pdf, removed: &BTreeSet<ObjectId>, changes: &mut Changes) {
    for object in pdf.file().objects() {
        let (owner, dict, stream) = match &object {
            Object::Dict(dict) => (dict.obj_id(), dict.clone(), None),
            Object::Stream(stream) => (Some(stream.obj_id()), stream.dict().clone(), Some(stream)),
            _ => continue,
        };
        let (Some(owner), Some(specs)) = (owner, dict.get::<Array<'_>>(b"AF")) else {
            continue;
        };
        let names = |item: &MaybeRef<Object<'_>>| {
            let Some(spec) = pdf.resolve(item).and_then(Object::into_dict) else {
                return false;
            };
            spec.embedded_file()
                .is_some_and(|(stream, _)| removed.contains(&stream.obj_id()))
        };
        let items: Vec<MaybeRef<Object<'_>>> = specs.raw_iter().collect();
        if !items.iter().any(names) {
            continue;
        }
        let kept: Vec<Value> = items
            .iter()
            .filter(|item| !names(item))
            .map(value)
            .collect();
        let empty = kept.is_empty();
        if let Some(array) = dict.reference(b"AF") {
            changes.replace(array, Value::Array(kept.clone()));
            if !empty {
                continue;
            }
        }
        let written = changes.entry(owner, || match stream {
            Some(stream) => Value::from(stream_value(stream)),
            None => Value::Dict(dict_value(&dict)),
        });
        if let Some(dict) = written.as_dict_mut() {
            if empty {
                dict.remove(b"AF");
            } else {
                dict.set("AF", Value::Array(kept));
            }
        }
    }
}
