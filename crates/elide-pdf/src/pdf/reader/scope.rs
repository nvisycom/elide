//! [`Scope`]: the resource dictionaries a content stream's names resolve in.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::pdf::reader::DocumentExt;

/// The resource dictionaries a content stream's names (`Tf`, `Do`, `gs`,
/// `scn`, `BDC`, ...) resolve in, nearest first: a page's own and those it
/// inherits, or the resources of a form, tiling pattern or Type3 font.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scope<'a>(Vec<&'a Dictionary>);

/// How many page-tree levels a page's resources are looked for in.
const MAX_TREE_DEPTH: usize = 64;

impl<'a> Scope<'a> {
    /// A scope of `resources`, nearest first.
    pub(crate) fn new(resources: impl IntoIterator<Item = &'a Dictionary>) -> Self {
        Self(resources.into_iter().collect())
    }

    /// The scope of page `page`: its `/Resources`, else the nearest page-tree
    /// ancestor's, taken whole (7.7.3.4): an entry is inherited as a value,
    /// so a page's own dictionary, even an empty one, hides its ancestors'.
    /// Empty when no dictionary is found.
    pub(crate) fn page(doc: &'a Document, page: ObjectId) -> Self {
        let mut node = doc.get_dictionary(page).ok();
        for _ in 0..MAX_TREE_DEPTH {
            let Some(dict) = node else {
                break;
            };
            if let Ok(resources) = dict.get(b"Resources") {
                return Self(doc.resolve_dict(resources).into_iter().collect());
            }
            node = dict
                .get(b"Parent")
                .ok()
                .and_then(|parent| doc.resolve_dict(parent));
        }
        Self::default()
    }

    /// The resources of the content `dict` (a form, tiling pattern or Type3
    /// font): its `/Resources`, when that is a dictionary it can use.
    pub(crate) fn own(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
        dict.get(b"Resources")
            .ok()
            .and_then(|resources| doc.resolve_dict(resources))
    }

    /// The scope the content `dict` drawn in this one resolves its names in:
    /// its own resources, else this scope.
    pub(crate) fn inner(&self, doc: &'a Document, dict: &'a Dictionary) -> Self {
        match Self::own(doc, dict) {
            Some(own) => Self(vec![own]),
            None => self.clone(),
        }
    }

    /// The entry `name` of the `category` sub-dictionary (`/Font`,
    /// `/XObject`, ...) of the nearest resource dictionary that has it. A
    /// null entry, or one referring to nothing, is no entry (7.3.9).
    pub(crate) fn lookup(
        &self,
        doc: &'a Document,
        category: &[u8],
        name: &[u8],
    ) -> Option<&'a Object> {
        self.0.iter().find_map(|resources| {
            let entries = resources
                .get(category)
                .ok()
                .and_then(|entries| doc.resolve_dict(entries))?;
            entries
                .get(name)
                .ok()
                .filter(|entry| !matches!(doc.resolve(entry), None | Some(Object::Null)))
        })
    }

    /// Every entry of the `category` sub-dictionaries, by name, each the
    /// nearest of its name; a null entry is none.
    pub(crate) fn entries(
        &self,
        doc: &'a Document,
        category: &[u8],
    ) -> BTreeMap<&'a [u8], &'a Object> {
        let mut out = BTreeMap::new();
        for resources in &self.0 {
            let entries = resources
                .get(category)
                .ok()
                .and_then(|entries| doc.resolve_dict(entries));
            for (name, value) in entries.into_iter().flat_map(Dictionary::iter) {
                if !matches!(doc.resolve(value), None | Some(Object::Null)) {
                    out.entry(name.as_slice()).or_insert(value);
                }
            }
        }
        out
    }

    /// The resource dictionaries, nearest first.
    pub(crate) fn dictionaries(&self) -> &[&'a Dictionary] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    /// A page's own `/Resources`, even empty, is its scope whole; one with
    /// none inherits its parent's.
    #[test]
    fn a_page_inherits_resources_whole() {
        let mut doc = Document::with_version("1.7");
        let parent_resources = doc.add_object(dictionary! {
            "ColorSpace" => dictionary! { "DefaultRGB" => "DeviceGray" },
        });
        let pages = doc.new_object_id();
        let own = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "Resources" => dictionary! {},
        });
        let inheriting = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![own.into(), inheriting.into()], "Count" => 2,
                "Resources" => parent_resources,
            }),
        );
        let lookup = |page| {
            Scope::page(&doc, page)
                .lookup(&doc, b"ColorSpace", b"DefaultRGB")
                .is_some()
        };
        assert!(!lookup(own));
        assert!(lookup(inheriting));
    }
}
