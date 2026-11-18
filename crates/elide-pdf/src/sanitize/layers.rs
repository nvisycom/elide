//! Optional-content layers: which the document's default configuration hides.
//!
//! A layer (an optional-content group) the default configuration turns off is
//! content a reader does not see, so the rebuild cuts what it marks: each
//! marked-content sequence tagged with it (`/OC /name BDC … EMC`) and each
//! XObject that belongs to it. A visible layer's content stays, as plain page
//! content.

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::pdf::reader::{DictionaryExt, DocumentExt};

/// How deep a visibility expression (`/VE`) may nest before it is refused.
const MAX_EXPRESSION_DEPTH: usize = 32;

/// The hidden layers of a document, and the reads that decide whether a piece
/// of optional content belongs to one.
pub(super) struct Layers<'a> {
    doc: &'a Document,
    hidden: HashSet<ObjectId>,
}

impl<'a> Layers<'a> {
    /// The layers of `doc`.
    pub(super) fn read(doc: &'a Document) -> Self {
        Self {
            doc,
            hidden: hidden_groups(doc),
        }
    }

    /// Whether the content `dict` belongs to (its `/OC`, if any) is hidden by
    /// the default configuration.
    pub(super) fn hides(&self, dict: &Dictionary) -> bool {
        dict.get(b"OC")
            .is_ok_and(|membership| self.is_hidden(membership))
    }

    /// Whether the optional-content `membership` (a group, or a membership
    /// dictionary over groups) is hidden by the default configuration.
    pub(super) fn is_hidden(&self, membership: &Object) -> bool {
        if let Object::Reference(id) = membership
            && self.hidden.contains(id)
        {
            return true;
        }
        let Some(dict) = self.doc.resolve_dict(membership) else {
            return false;
        };
        if !dict.is_type(self.doc, b"OCMD") {
            return false;
        }
        if let Ok(expression) = dict.get(b"VE") {
            return !self.expression_visible(expression, 0);
        }
        // A null or deleted entry names no group, so it is no member.
        let is_group = |group: &&Object| self.doc.resolve_dict(group).is_some();
        let Ok(members) = dict.get(b"OCGs") else {
            return false;
        };
        let groups: Vec<bool> = match self.doc.resolve_array(members) {
            Some(groups) => groups
                .iter()
                .filter(is_group)
                .map(|g| self.group_hidden(g))
                .collect(),
            None if is_group(&members) => vec![self.group_hidden(members)],
            None => return false,
        };
        if groups.is_empty() {
            return false;
        }
        let policy = dict.name(self.doc, b"P").unwrap_or(b"AnyOn");
        match policy {
            b"AllOn" => groups.iter().any(|&hidden| hidden),
            b"AnyOff" => groups.iter().all(|&hidden| !hidden),
            b"AllOff" => groups.iter().any(|&hidden| !hidden),
            _ => groups.iter().all(|&hidden| hidden),
        }
    }

    /// Whether the single group `group` is hidden.
    fn group_hidden(&self, group: &Object) -> bool {
        group
            .as_reference()
            .is_ok_and(|id| self.hidden.contains(&id))
    }

    /// Whether the visibility expression `expression` (`[/And …]`, `[/Or …]`,
    /// `[/Not …]` over groups) evaluates visible. An expression too deep or
    /// malformed to evaluate, anywhere in it, counts as visible as a whole:
    /// its content then stays in place and is extracted and redacted like
    /// any other.
    fn expression_visible(&self, expression: &Object, depth: usize) -> bool {
        self.evaluate(expression, depth).unwrap_or(true)
    }

    /// The visibility `expression` evaluates to, or `None` when it is too
    /// deep or malformed to evaluate.
    fn evaluate(&self, expression: &Object, depth: usize) -> Option<bool> {
        if depth > MAX_EXPRESSION_DEPTH {
            return None;
        }
        let expression = match expression {
            Object::Reference(id) => match self.doc.get_object(*id).ok()? {
                Object::Array(array) => array,
                Object::Dictionary(_) => return Some(!self.group_hidden(expression)),
                _ => return None,
            },
            Object::Array(array) => array,
            _ => return None,
        };
        let (Object::Name(operator), operands) = expression.split_first()? else {
            return None;
        };
        let visible = operands
            .iter()
            .map(|o| self.evaluate(o, depth + 1))
            .collect::<Option<Vec<bool>>>()?;
        match (operator.as_slice(), visible.as_slice()) {
            (b"And", [_, ..]) => Some(visible.iter().all(|&v| v)),
            (b"Or", [_, ..]) => Some(visible.iter().any(|&v| v)),
            (b"Not", [one]) => Some(!one),
            _ => None,
        }
    }
}

/// The layers hidden by the document's default configuration: every group
/// listed `/OFF`, or, when the base state is `/OFF`, every group not listed
/// `/ON`.
fn hidden_groups(doc: &Document) -> HashSet<ObjectId> {
    let Some(properties) = doc
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"OCProperties").ok())
        .and_then(|p| doc.resolve_dict(p))
    else {
        return HashSet::new();
    };
    let Some(config) = properties.get(b"D").ok().and_then(|d| doc.resolve_dict(d)) else {
        return HashSet::new();
    };
    let ids = |key: &[u8], dict: &Dictionary| -> HashSet<ObjectId> {
        dict.get(key)
            .ok()
            .and_then(|array| doc.resolve_array(array))
            .map(|array| array.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default()
    };
    let off = if config.name(doc, b"BaseState") == Some(b"OFF") {
        let on = ids(b"ON", config);
        ids(b"OCGs", properties)
            .into_iter()
            .filter(|id| !on.contains(id))
            .collect()
    } else {
        ids(b"OFF", config)
    };
    // A group whose intent the configuration does not share has no effect
    // on visibility (8.11.2.1), so it hides nothing.
    let viewing = intents(doc, config);
    off.into_iter()
        .filter(|&id| {
            let group = doc.get_dictionary(id).ok();
            let intended = group.map(|group| intents(doc, group)).unwrap_or_default();
            viewing.contains(&b"All"[..].to_vec())
                || intended.contains(&b"All"[..].to_vec())
                || intended.iter().any(|intent| viewing.contains(intent))
        })
        .collect()
}

/// The intents `dict` (a group or a configuration) names in `/Intent`: a
/// name or an array of names, `/View` when it names none.
fn intents(doc: &Document, dict: &Dictionary) -> Vec<Vec<u8>> {
    let named = match dict
        .get(b"Intent")
        .ok()
        .and_then(|intent| doc.resolve(intent))
    {
        Some(Object::Name(name)) => vec![name.clone()],
        Some(Object::Array(names)) => names
            .iter()
            .filter_map(|name| doc.resolve(name)?.as_name().ok().map(<[u8]>::to_vec))
            .collect(),
        _ => Vec::new(),
    };
    if named.is_empty() {
        vec![b"View".to_vec()]
    } else {
        named
    }
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    /// A document whose default configuration turns `layer` off, with the
    /// group's and the configuration's intents as given.
    fn document(group: Dictionary, config: Dictionary) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.7");
        let layer = doc.add_object(group);
        let mut default = dictionary! { "OFF" => vec![layer.into()] };
        default.extend(&config);
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "OCProperties" => dictionary! { "OCGs" => vec![layer.into()], "D" => default },
        });
        doc.trailer.set("Root", catalog);
        (doc, layer)
    }

    /// A malformed operand makes the whole expression count as visible: a
    /// `/Not` does not turn its fallback into hidden.
    #[test]
    fn a_malformed_expression_is_visible_as_a_whole() {
        let (doc, layer) = document(dictionary! { "Type" => "OCG" }, Dictionary::new());
        let layers = Layers::read(&doc);
        let membership = |expression: Object| {
            Object::Dictionary(dictionary! { "Type" => "OCMD", "VE" => expression })
        };
        let not = |operand: Object| Object::Array(vec![Object::Name(b"Not".to_vec()), operand]);
        assert!(!layers.is_hidden(&membership(not(5.into()))));
        assert!(!layers.is_hidden(&membership(not(layer.into()))));
        assert!(layers.is_hidden(&membership(not(not(layer.into())))));
    }

    /// A group meant only for design has no effect on viewing, so being off
    /// hides nothing; one meant for viewing does.
    #[test]
    fn a_group_of_another_intent_hides_nothing() {
        let (doc, layer) = document(
            dictionary! { "Type" => "OCG", "Intent" => "Design" },
            Dictionary::new(),
        );
        assert!(!Layers::read(&doc).is_hidden(&Object::Reference(layer)));
        let (doc, layer) = document(
            dictionary! { "Type" => "OCG", "Intent" => vec!["Design".into(), "View".into()] },
            Dictionary::new(),
        );
        assert!(Layers::read(&doc).is_hidden(&Object::Reference(layer)));
        let (doc, layer) = document(
            dictionary! { "Type" => "OCG", "Intent" => "Design" },
            dictionary! { "Intent" => "All" },
        );
        assert!(Layers::read(&doc).is_hidden(&Object::Reference(layer)));
    }
}
