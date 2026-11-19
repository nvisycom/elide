//! Links to a place in the document: each reduced to where it is and where
//! it goes, its destination resolved to the copied page.

use std::collections::HashMap;

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use lopdf::{Dictionary, Document, Object};

use super::copy::At;
use super::keys::Kind;
use super::{Builder, MAX_NESTING, record};
use crate::pdf::reader::{DictionaryExt, DocumentExt, NameTree, Scope};

impl<'a> Builder<'a> {
    /// The annotation `dict` as a link to a place in this document, reduced
    /// to where it is and where it goes; `None` for any other annotation,
    /// recorded by what it is, and for a link in a layer hidden by default,
    /// which is no link to follow.
    pub(super) fn link(&mut self, dict: &'a Dictionary) -> Result<Option<Object>> {
        if self.layers.hides(dict) {
            self.record(HiddenContent::Layer);
            return Ok(None);
        }
        let Some(dest) = self.internal_destination(dict) else {
            self.record(record::annotation(self.source, dict));
            return Ok(None);
        };
        let mut link = Dictionary::from_iter([
            ("Type", Object::Name(b"Annot".to_vec())),
            ("Subtype", Object::Name(b"Link".to_vec())),
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
            if let Ok(value) = dict.get(key)
                && let Some(value) = self.copy(kind, value, At::top(&scope))?
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
        if dict.iter().any(|(key, _)| !kept.contains(&key.as_slice())) {
            self.record(HiddenContent::Annotation);
        }
        Ok(Some(Object::Reference(self.out.add_object(link))))
    }

    /// Where the annotation `dict` goes, as an explicit destination on a
    /// copied page, when it is a link to a place in this document: a `/Link`
    /// with a `/Dest`, or whose action is a `/GoTo` that chains no further
    /// action (a `/Next` can run script after the jump).
    pub(super) fn internal_destination(&self, dict: &Dictionary) -> Option<Object> {
        let source = self.source;
        if !dict.is_subtype(source, b"Link") {
            return None;
        }
        let target = match dict.get(b"A").ok().and_then(|a| source.resolve_dict(a)) {
            Some(action) => {
                let go_to = action
                    .get(b"S")
                    .ok()
                    .and_then(|s| source.resolve(s))
                    .and_then(|s| s.as_name().ok())
                    == Some(b"GoTo");
                if !go_to || action.get(b"Next").is_ok() {
                    return None;
                }
                action.get(b"D").ok()?
            }
            None => dict.get(b"Dest").ok()?,
        };
        self.destination(target, 0)
    }

    /// The explicit destination `dest` names, its page mapped to the copy.
    pub(super) fn destination(&self, dest: &Object, depth: usize) -> Option<Object> {
        if depth > MAX_NESTING {
            return None;
        }
        let source = self.source;
        match source.resolve(dest)? {
            Object::Array(items) => {
                let (page, view) = items.split_first()?;
                let page = self.pages.get(&page.as_reference().ok()?)?;
                let mut out = vec![Object::Reference(*page)];
                for item in view {
                    match source.resolve(item)? {
                        value @ (Object::Name(_)
                        | Object::Integer(_)
                        | Object::Real(_)
                        | Object::Null) => out.push(value.clone()),
                        _ => return None,
                    }
                }
                Some(Object::Array(out))
            }
            Object::Dictionary(dict) => self.destination(dict.get(b"D").ok()?, depth + 1),
            Object::Name(name) => self.destination(self.named(name)?, depth + 1),
            Object::String(name, _) => {
                self.destination(self.destinations.get(name.as_slice())?, depth + 1)
            }
            _ => None,
        }
    }

    /// The destination the name `name` names, in the catalog's `/Dests`; a
    /// string names one in the `/Names` tree of destinations instead
    /// (12.3.2.4).
    fn named(&self, name: &[u8]) -> Option<&'a Object> {
        let source = self.source;
        source
            .catalog()
            .ok()?
            .get(b"Dests")
            .ok()
            .and_then(|d| source.resolve_dict(d))?
            .get(name)
            .ok()
    }
}

/// Each name in `source`'s `/Names` tree of destinations, with the first
/// destination it names; a node that cannot be read leaves out only the
/// destinations under it.
pub(super) fn destinations(source: &Document) -> HashMap<&[u8], &Object> {
    let mut out = HashMap::new();
    for (key, dest) in NameTree::read(source, b"Dests").pairs {
        if let Object::String(key, _) = key {
            out.entry(key.as_slice()).or_insert(dest);
        }
    }
    out
}
