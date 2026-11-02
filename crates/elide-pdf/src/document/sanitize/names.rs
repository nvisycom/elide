//! Neutral names for a PDF's embedded files: a file's name is shown in a
//! viewer's attachment panel but never reaches detection, so it is replaced.

use std::collections::BTreeSet;

use elide_core::Result;
use lopdf::{Document, Object, ObjectId, decode_text_string};

use crate::document::attachment::tree::{Tree, node_dict, write_leaf};
use crate::document::object::{DictionaryExt, ObjectExt};

/// The neutral name every embedded file is given: its file name is visible in
/// a viewer's attachment panel but never reaches detection.
const NEUTRAL_PREFIX: &str = "attachment-";

/// Give every embedded file a neutral name, `attachment-N` with its original
/// extension (which the codec decodes it by), and drop the portfolio and
/// related-file entries that name it too, returning how many file
/// specifications changed.
///
/// A file's name (`/F`, `/UF`, and its key in the attachment tree) is shown to
/// a reader but never extracted, so it cannot be redacted and is replaced. The
/// tree is rebuilt as one sorted leaf, since renamed keys would break its
/// order and limits. A name that is already neutral stays, so a second pass
/// changes nothing.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the attachment tree cannot be read.
pub(super) fn neutralize(doc: &mut Document) -> Result<usize> {
    let specs: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, object)| object.dict().is_some_and(|dict| dict.get(b"EF").is_ok()))
        .map(|(&id, _)| id)
        .collect();
    let mut edited_specs: BTreeSet<ObjectId> = BTreeSet::new();
    let mut names: Vec<(ObjectId, String)> = Vec::new();
    for (index, id) in specs.into_iter().enumerate() {
        let Some(spec) = doc.get_object_mut(id).ok().and_then(ObjectExt::dict_mut) else {
            continue;
        };
        let current = spec.file_name().unwrap_or_default();
        let name = if is_neutral(&current) {
            current
        } else {
            neutral_name(index + 1, &current)
        };
        let mut edited = false;
        for key in [&b"F"[..], b"UF"] {
            let already = spec
                .get(key)
                .ok()
                .and_then(|name| decode_text_string(name).ok())
                .is_some_and(|value| value == name);
            if !already {
                spec.set(key, Object::string_literal(name.clone()));
                edited = true;
            }
        }
        for key in [&b"CI"[..], b"RF"] {
            edited |= spec.remove(key).is_some();
        }
        if edited {
            edited_specs.insert(id);
        }
        names.push((id, name));
    }
    let renamed_keys = rebuild_tree(doc, &names, &edited_specs)?;
    Ok(edited_specs.len() + renamed_keys)
}

/// Rebuild the attachment tree as one leaf of its entries sorted by key, each
/// keyed by its file specification's neutral name from `names`, when any key
/// is not already that name. Returns how many keys changed whose file
/// specification is not among the `edited` ones (already counted).
fn rebuild_tree(
    doc: &mut Document,
    names: &[(ObjectId, String)],
    edited: &BTreeSet<ObjectId>,
) -> Result<usize> {
    let tree = Tree::read(doc)?;
    if tree.nodes.is_empty() {
        return Ok(0);
    }
    let mut renamed = 0;
    let mut any_renamed = false;
    let pairs: Vec<(String, Object)> = tree
        .nodes
        .iter()
        .filter_map(|&node| {
            node_dict(doc, node)?
                .get(b"Names")
                .and_then(Object::as_array)
                .ok()
        })
        .flat_map(|array| array.chunks_exact(2))
        .map(|pair| {
            let current = decode_text_string(&pair[0]).unwrap_or_default();
            let spec = match &pair[1] {
                Object::Reference(id) => names.iter().find(|(spec, _)| spec == id),
                _ => None,
            };
            let key = spec.map_or_else(|| current.clone(), |(_, name)| name.clone());
            if key != current {
                any_renamed = true;
                renamed += usize::from(spec.is_none_or(|(id, _)| !edited.contains(id)));
            }
            (key, pair[1].clone())
        })
        .collect();
    if !any_renamed {
        return Ok(0);
    }
    write_leaf(doc, pairs);
    Ok(renamed)
}

/// Whether `name` is already neutral: `attachment-N`, with or without an
/// extension.
fn is_neutral(name: &str) -> bool {
    let stem = name.split_once('.').map_or(name, |(stem, _)| stem);
    stem.strip_prefix(NEUTRAL_PREFIX)
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

/// The neutral name of the `index`th embedded file, keeping the extension of
/// its `original` name when it has a plain one.
fn neutral_name(index: usize, original: &str) -> String {
    let extension = original
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| {
            (1..=8).contains(&extension.len())
                && extension.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    match extension {
        Some(extension) => format!("{NEUTRAL_PREFIX}{index}.{}", extension.to_ascii_lowercase()),
        None => format!("{NEUTRAL_PREFIX}{index}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Pdf;
    use crate::document::attachment::with_attachments;
    use crate::document::object::DocumentExt;

    #[test]
    fn attached_file_names_are_made_neutral() {
        let bytes = with_attachments(&[
            ("Jane_Roe_passport.pdf", b"pdf bytes"),
            ("notes.txt", b"Jane Roe"),
        ]);
        let mut doc = Document::load_mem(&bytes).unwrap();
        assert_eq!(neutralize(&mut doc).unwrap(), 2);
        assert_eq!(neutralize(&mut doc).unwrap(), 0);

        // The replaced tree nodes are unreachable; the save prunes them.
        let out = doc.save_pruned().unwrap();
        assert!(!out.windows(8).any(|w| w == b"Jane_Roe"));
        let mut names: Vec<(String, Vec<u8>)> = Pdf::open(&out)
            .unwrap()
            .attachments()
            .unwrap()
            .into_iter()
            .map(|a| (a.name, a.bytes.to_vec()))
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                ("attachment-1.pdf".to_owned(), b"pdf bytes".to_vec()),
                ("attachment-2.txt".to_owned(), b"Jane Roe".to_vec()),
            ]
        );
    }
}
