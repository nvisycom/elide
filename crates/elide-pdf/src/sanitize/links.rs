//! Links to a place in the document: each reduced to where it is and where
//! it goes, its destination resolved to the copied page.

use std::collections::HashMap;

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use hayro_syntax::object::{Dict, MaybeRef, Object};

use super::copy::At;
use super::keys::Kind;
use super::{Builder, MAX_NESTING, record};
use crate::pdf::Pdf;
use crate::pdf::object::{Dictionary, Value};
use crate::pdf::reader::{DictExt, NameTree, Scope, id};
use crate::pdf::writer::object_value;

impl<'a> Builder<'a> {
    /// The annotation `dict` as a link to a place in this document, reduced
    /// to where it is and where it goes; `None` for any other annotation,
    /// recorded by what it is, and for a link in a layer hidden by default,
    /// which is no link to follow.
    pub(super) fn link(&mut self, dict: &Dict<'a>) -> Result<Option<Value>> {
        if self.layers.hides(dict) {
            self.record(HiddenContent::Layer);
            return Ok(None);
        }
        let Some(dest) = self.internal_destination(dict) else {
            self.record(record::annotation(dict));
            return Ok(None);
        };
        let mut link = Dictionary::from_iter([
            ("Type", Value::name("Annot")),
            ("Subtype", Value::name("Link")),
            ("Dest", dest),
        ]);
        let scope = Scope::default();
        let fields: [(&[u8], Kind); 7] = [
            (b"Rect", Kind::Value),
            (b"Border", Kind::Value),
            (b"BS", Kind::BorderStyle),
            (b"C", Kind::Value),
            (b"H", Kind::Value),
            (b"QuadPoints", Kind::Value),
            (b"F", Kind::Value),
        ];
        for (key, kind) in fields {
            if let Some(value) = dict.raw(key)
                && let Some(value) = self.copy(kind, &value, At::top(&scope))?
            {
                link.set(key, value);
            }
        }
        let kept = [
            &b"Type"[..],
            b"Subtype",
            b"Rect",
            b"Border",
            b"BS",
            b"C",
            b"H",
            b"QuadPoints",
            b"F",
            b"Dest",
            b"A",
            b"P",
        ];
        if dict.keys().any(|key| !kept.contains(&&*key)) {
            self.record(HiddenContent::Annotation);
        }
        Ok(Some(Value::Ref(self.out.add(link))))
    }

    /// Where the annotation `dict` goes, as an explicit destination on a
    /// copied page, when it is a link to a place in this document: a `/Link`
    /// with a `/Dest`, or whose action is a `/GoTo` that chains no further
    /// action (a `/Next` can run script after the jump).
    pub(super) fn internal_destination(&self, dict: &Dict<'a>) -> Option<Value> {
        if !dict.is_subtype(b"Link") {
            return None;
        }
        let target = match dict.get::<Dict<'a>>(b"A") {
            Some(action) => {
                let go_to = action.name(b"S").is_some_and(|s| &*s == b"GoTo");
                if !go_to || action.contains_key(b"Next") {
                    return None;
                }
                action.raw(b"D")?
            }
            None => dict.raw(b"Dest")?,
        };
        self.destination(&target, 0)
    }

    /// The explicit destination `dest` names, its page mapped to the copy.
    pub(super) fn destination(&self, dest: &MaybeRef<Object<'a>>, depth: usize) -> Option<Value> {
        if depth > MAX_NESTING {
            return None;
        }
        match self.source.resolve(dest)? {
            Object::Array(items) => {
                let items: Vec<MaybeRef<Object<'a>>> = items.raw_iter().collect();
                let (page, view) = items.split_first()?;
                let MaybeRef::Ref(page) = page else {
                    return None;
                };
                let page = self.pages.get(&id(*page))?;
                let mut out = vec![Value::Ref(*page)];
                for item in view {
                    match self.source.resolve(item)? {
                        value @ (Object::Name(_) | Object::Number(_) | Object::Null(_)) => {
                            out.push(object_value(&value));
                        }
                        _ => return None,
                    }
                }
                Some(Value::Array(out))
            }
            Object::Dict(dict) => self.destination(&dict.raw(b"D")?, depth + 1),
            Object::Name(name) => self.destination(&self.named(&name)?, depth + 1),
            Object::String(name) => {
                let named = self.destinations.get(name.as_bytes())?.clone();
                self.destination(&MaybeRef::NotRef(named), depth + 1)
            }
            _ => None,
        }
    }

    /// The destination the name `name` names, in the catalog's `/Dests`; a
    /// string names one in the `/Names` tree of destinations instead
    /// (12.3.2.4).
    fn named(&self, name: &[u8]) -> Option<MaybeRef<Object<'a>>> {
        self.source
            .catalog()
            .ok()?
            .get::<Dict<'a>>(b"Dests")?
            .raw(name)
    }
}

/// Each name in `source`'s `/Names` tree of destinations, with the first
/// destination it names; a node that cannot be read leaves out only the
/// destinations under it.
pub(super) fn destinations(source: &Pdf) -> HashMap<Vec<u8>, Object<'_>> {
    let mut out = HashMap::new();
    for pair in NameTree::read(source, b"Dests").pairs {
        if let Object::String(key) = pair.key {
            out.entry(key.as_bytes().to_vec()).or_insert(pair.value);
        }
    }
    out
}
