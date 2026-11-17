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

    /// Whether the optional-content `membership` (a group, or a membership
    /// dictionary over groups) is hidden by the default configuration.
    pub(super) fn is_hidden(&self, membership: &Object, depth: usize) -> bool {
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
            return !self.expression_visible(expression, depth);
        }
        let groups: Vec<bool> = match dict.get(b"OCGs") {
            Ok(Object::Array(groups)) => groups.iter().map(|g| self.group_hidden(g)).collect(),
            Ok(group) => vec![self.group_hidden(group)],
            Err(_) => return false,
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
    /// malformed to evaluate counts as visible: its content then stays in
    /// place and is extracted and redacted like any other.
    fn expression_visible(&self, expression: &Object, depth: usize) -> bool {
        if depth > MAX_EXPRESSION_DEPTH {
            return true;
        }
        let expression = match expression {
            Object::Reference(id) => match self.doc.get_object(*id) {
                Ok(Object::Array(array)) => array.clone(),
                _ => return !self.group_hidden(expression),
            },
            Object::Array(array) => array.clone(),
            _ => return true,
        };
        let Some((Object::Name(operator), operands)) = expression.split_first() else {
            return true;
        };
        let mut visible = operands
            .iter()
            .map(|o| self.expression_visible(o, depth + 1));
        match operator.as_slice() {
            b"And" => visible.all(|v| v),
            b"Or" => visible.any(|v| v),
            b"Not" => !visible.next().unwrap_or(true),
            _ => true,
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
    if config.name(doc, b"BaseState") == Some(b"OFF") {
        let on = ids(b"ON", config);
        ids(b"OCGs", properties)
            .into_iter()
            .filter(|id| !on.contains(id))
            .collect()
    } else {
        ids(b"OFF", config)
    }
}
