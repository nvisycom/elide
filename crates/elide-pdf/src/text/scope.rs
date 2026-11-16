//! Resource scopes: the dictionaries a content stream's `Tf` and `Do` names
//! resolve in, and the lookups through them.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object, ObjectId};

/// Collect the `/Font` entries of a resource scope (inline plus any referenced
/// resource dictionaries), name -> font dictionary.
pub(super) fn scope_fonts<'a>(
    doc: &'a Document,
    scope: &Scope<'a>,
) -> BTreeMap<Vec<u8>, &'a Dictionary> {
    let mut out = BTreeMap::new();
    let mut add = |resources: &'a Dictionary| {
        if let Ok(font) = resources.get(b"Font")
            && let Some(font_dict) = dict_or_ref(doc, font)
        {
            for (name, value) in font_dict.iter() {
                let font = match value {
                    Object::Reference(id) => doc.get_dictionary(*id).ok(),
                    Object::Dictionary(dict) => Some(dict),
                    _ => None,
                };
                if let Some(font) = font {
                    out.entry(name.clone()).or_insert(font);
                }
            }
        }
    };
    if let Some(inline) = scope.inline {
        add(inline);
    }
    for &id in scope.referenced {
        if let Ok(resources) = doc.get_dictionary(id) {
            add(resources);
        }
    }
    out
}

/// Resolve the object id of the XObject named `name` in `scope`'s `/XObject`
/// dictionary, if it is an indirect reference.
pub(super) fn scope_xobject_id(doc: &Document, scope: &Scope<'_>, name: &[u8]) -> Option<ObjectId> {
    let resource_dicts = scope.inline.into_iter().chain(
        scope
            .referenced
            .iter()
            .filter_map(|&id| doc.get_dictionary(id).ok()),
    );
    for resources in resource_dicts {
        if let Ok(xobjects) = resources.get(b"XObject")
            && let Some(xobjects) = dict_or_ref(doc, xobjects)
            && let Ok(entry) = xobjects.get(name)
            && let Ok(id) = entry.as_reference()
        {
            return Some(id);
        }
    }
    None
}

/// Resolve an object that is a dictionary, inline or by reference.
pub(super) fn dict_or_ref<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

/// A resource dictionary scope a stream's `Tf`/`Do` names resolve in: an inline
/// dictionary and any referenced resource dictionaries (a page can inherit
/// several through its tree).
#[derive(Clone, Copy)]
pub(super) struct Scope<'a> {
    pub(super) inline: Option<&'a Dictionary>,
    pub(super) referenced: &'a [ObjectId],
}
