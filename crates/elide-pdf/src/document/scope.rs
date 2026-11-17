//! [`Scope`]: the resource dictionaries a content stream's names resolve in.

use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::document::object::DocumentExt;

/// The resource dictionaries a content stream's names (`Tf`, `Do`, `gs`,
/// `scn`, `BDC`, ...) resolve in, nearest first: a page's own and those it
/// inherits, or the resources of a form, tiling pattern or Type3 font.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scope<'a>(Vec<&'a Dictionary>);

impl<'a> Scope<'a> {
    /// A scope of `resources`, nearest first.
    pub(crate) fn new(resources: impl IntoIterator<Item = &'a Dictionary>) -> Self {
        Self(resources.into_iter().collect())
    }

    /// The scope of page `page`: its own resources, then those it inherits,
    /// nearest first; empty when the page cannot be read.
    pub(crate) fn page(doc: &'a Document, page: ObjectId) -> Self {
        let Ok((own, inherited)) = doc.get_page_resources(page) else {
            return Self::default();
        };
        let inherited = inherited
            .into_iter()
            .filter_map(|id| doc.get_dictionary(id).ok());
        Self::new(own.into_iter().chain(inherited))
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
    /// `/XObject`, ...) of the nearest resource dictionary that has it.
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
            entries.get(name).ok()
        })
    }

    /// Every entry of the `category` sub-dictionaries, by name, each the
    /// nearest of its name.
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
                out.entry(name.as_slice()).or_insert(value);
            }
        }
        out
    }

    /// The resource dictionaries, nearest first.
    pub(crate) fn dictionaries(&self) -> &[&'a Dictionary] {
        &self.0
    }
}
