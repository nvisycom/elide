//! [`Scope`]: the resource dictionaries a content stream's names resolve in.

use std::collections::BTreeMap;

use hayro_syntax::object::{Dict, MaybeRef, Object};

use crate::pdf::reader::{DictExt, ObjectId, reference_to};

/// The resource dictionaries a content stream's names (`Tf`, `Do`, `gs`,
/// `scn`, `BDC`, ...) resolve in, nearest first: a page's own and those it
/// inherits, or the resources of a form, tiling pattern or Type3 font.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scope<'a>(Vec<Dict<'a>>);

/// A resource a scope names: its value, and the object it is when it is
/// one of its own.
#[derive(Debug, Clone)]
pub(crate) struct Entry<'a> {
    pub(crate) id: Option<ObjectId>,
    pub(crate) value: Object<'a>,
}

impl<'a> Entry<'a> {
    /// The entry as the scope holds it: a reference to its object, or its
    /// value in place.
    pub(crate) fn held(self) -> MaybeRef<Object<'a>> {
        match self.id {
            Some(id) => reference_to(id),
            None => MaybeRef::NotRef(self.value),
        }
    }
}

/// How many page-tree levels a page's resources are looked for in.
const MAX_TREE_DEPTH: usize = 64;

impl<'a> Scope<'a> {
    /// A scope of `resources`, nearest first.
    pub(crate) fn new(resources: impl IntoIterator<Item = Dict<'a>>) -> Self {
        Self(resources.into_iter().collect())
    }

    /// The scope of the page `page`: its `/Resources`, else the nearest
    /// page-tree ancestor's, taken whole (7.7.3.4): an entry is inherited as
    /// a value, so a page's own dictionary, even an empty one, hides its
    /// ancestors'. Empty when no dictionary is found.
    pub(crate) fn page(page: &Dict<'a>) -> Self {
        let mut node = Some(page.clone());
        for _ in 0..MAX_TREE_DEPTH {
            let Some(dict) = node else {
                break;
            };
            // A null or dangling entry is absent, so inheritance goes on.
            match dict.get::<Object<'a>>(b"Resources") {
                None | Some(Object::Null(_)) => {}
                Some(resources) => return Self(resources.into_dict().into_iter().collect()),
            }
            node = dict.get::<Dict<'a>>(b"Parent");
        }
        Self::default()
    }

    /// The resources of the content `dict` (a form, tiling pattern or Type3
    /// font): its `/Resources`, when that is a dictionary it can use.
    pub(crate) fn own(dict: &Dict<'a>) -> Option<Dict<'a>> {
        dict.get::<Dict<'a>>(b"Resources")
    }

    /// The scope the content `dict` drawn in this one resolves its names in:
    /// its own resources, else this scope.
    pub(crate) fn inner(&self, dict: &Dict<'a>) -> Self {
        match Self::own(dict) {
            Some(own) => Self(vec![own]),
            None => self.clone(),
        }
    }

    /// The entry `name` of the `category` sub-dictionary (`/Font`,
    /// `/XObject`, ...) of the nearest resource dictionary that has it. A
    /// null entry, or one referring to nothing, is no entry (7.3.9).
    pub(crate) fn lookup(&self, category: &[u8], name: &[u8]) -> Option<Entry<'a>> {
        self.0
            .iter()
            .find_map(|resources| entry(&resources.get::<Dict<'a>>(category)?, name))
    }

    /// Every entry of the `category` sub-dictionaries, by name, each the
    /// nearest of its name; a null entry is none.
    pub(crate) fn entries(&self, category: &[u8]) -> BTreeMap<Vec<u8>, Entry<'a>> {
        let mut out = BTreeMap::new();
        for resources in &self.0 {
            let Some(entries) = resources.get::<Dict<'a>>(category) else {
                continue;
            };
            for name in entries.keys() {
                if out.contains_key(&*name) {
                    continue;
                }
                if let Some(found) = entry(&entries, &name) {
                    out.insert(name.to_vec(), found);
                }
            }
        }
        out
    }

    /// The resource dictionaries, nearest first.
    pub(crate) fn dictionaries(&self) -> &[Dict<'a>] {
        &self.0
    }
}

/// The entry `name` of `entries`, `None` when it is null or refers to
/// nothing.
fn entry<'a>(entries: &Dict<'a>, name: &[u8]) -> Option<Entry<'a>> {
    let value = entries.get::<Object<'a>>(name)?;
    if matches!(value, Object::Null(_)) {
        return None;
    }
    Some(Entry {
        id: entries.reference(name),
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{held, read_id};
    use crate::pdf::object::{Value, dict};
    use crate::pdf::writer::Document;

    /// A page's own `/Resources`, even empty, is its scope whole; one with
    /// none, or a null one, inherits its parent's.
    #[test]
    fn a_page_inherits_resources_whole() {
        let mut doc = Document::new((1, 7));
        let parent_resources = doc.add(dict! {
            "ColorSpace" => dict! { "DefaultRGB" => "DeviceGray" },
        });
        let pages = doc.reserve();
        let own = doc.add(dict! {
            "Type" => "Page", "Parent" => pages, "Resources" => dict! {},
        });
        let inheriting = doc.add(dict! { "Type" => "Page", "Parent" => pages });
        let null = doc.add(dict! {
            "Type" => "Page", "Parent" => pages, "Resources" => Value::Null,
        });
        doc.set(
            pages,
            dict! {
                "Type" => "Pages", "Kids" => vec![own.into(), inheriting.into(), null.into()],
                "Count" => 3, "Resources" => parent_resources,
            },
        );
        let pdf = held(doc, &[own, inheriting, null]);
        let lookup = |page| {
            let page = pdf.object(read_id(page)).unwrap().into_dict().unwrap();
            Scope::page(&page)
                .lookup(b"ColorSpace", b"DefaultRGB")
                .is_some()
        };
        assert!(!lookup(own));
        assert!(lookup(inheriting));
        assert!(lookup(null));
    }
}
