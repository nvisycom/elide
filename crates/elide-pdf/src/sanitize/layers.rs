//! Optional-content layers: which the document's default configuration hides.
//!
//! A layer (an optional-content group) the default configuration turns off is
//! content a reader does not see, so the rebuild cuts what it marks: each
//! marked-content sequence tagged with it (`/OC /name BDC … EMC`) and each
//! XObject that belongs to it. A visible layer's content stays, as plain page
//! content.

use std::collections::HashSet;

use hayro_syntax::object::{Array, Dict, MaybeRef, Object};

use crate::pdf::Pdf;
use crate::pdf::reader::{DictExt, ObjectId, id};

/// How deep a visibility expression (`/VE`) may nest before it is refused.
const MAX_EXPRESSION_DEPTH: usize = 32;

/// How many operands one visibility expression is evaluated through, all
/// told, so a small expression sharing subexpressions does not expand into
/// a vast evaluation.
const MAX_EXPRESSION_WORK: usize = 4096;

/// The hidden layers of a document, and the reads that decide whether a piece
/// of optional content belongs to one.
pub(super) struct Layers<'a> {
    pdf: &'a Pdf,
    /// The groups hidden, or `None` when the configuration applies no
    /// visibility rule (its `/Intent` is an empty array), so everything
    /// shows.
    hidden: Option<HashSet<ObjectId>>,
}

impl<'a> Layers<'a> {
    /// The layers of `pdf`.
    pub(super) fn read(pdf: &'a Pdf) -> Self {
        Self {
            pdf,
            hidden: hidden_groups(pdf),
        }
    }

    /// Whether the content `dict` belongs to (its `/OC`, if any) is hidden by
    /// the default configuration.
    pub(super) fn hides(&self, dict: &Dict<'a>) -> bool {
        dict.raw(b"OC")
            .is_some_and(|membership| self.is_hidden(&membership))
    }

    /// Whether the optional-content `membership` (a group, or a membership
    /// dictionary over groups) is hidden by the default configuration.
    pub(super) fn is_hidden(&self, membership: &MaybeRef<Object<'a>>) -> bool {
        let Some(hidden) = &self.hidden else {
            return false;
        };
        if let MaybeRef::Ref(reference) = membership
            && hidden.contains(&id(*reference))
        {
            return true;
        }
        let Some(dict) = self.pdf.resolve(membership).and_then(Object::into_dict) else {
            return false;
        };
        if !dict.is_type(b"OCMD") {
            return false;
        }
        if let Some(expression) = dict.raw(b"VE") {
            return !self.expression_visible(&expression, 0);
        }
        let Some(members) = dict.raw(b"OCGs") else {
            return false;
        };
        // A null or deleted entry names no group, so it is no member.
        let groups: Vec<bool> = match self.pdf.resolve(&members) {
            Some(Object::Array(groups)) => groups
                .raw_iter()
                .filter(|group| self.is_group(group))
                .map(|group| self.group_hidden(&group))
                .collect(),
            Some(_) if self.is_group(&members) => vec![self.group_hidden(&members)],
            _ => return false,
        };
        if groups.is_empty() {
            return false;
        }
        let policy = dict.name(b"P");
        match policy.as_deref().unwrap_or(b"AnyOn") {
            b"AllOn" => groups.iter().any(|&hidden| hidden),
            b"AnyOff" => groups.iter().all(|&hidden| !hidden),
            b"AllOff" => groups.iter().any(|&hidden| !hidden),
            _ => groups.iter().all(|&hidden| hidden),
        }
    }

    /// Whether `item` is a dictionary, as a group is.
    fn is_group(&self, item: &MaybeRef<Object<'a>>) -> bool {
        matches!(self.pdf.resolve(item), Some(Object::Dict(_)))
    }

    /// Whether the single group `group` is hidden.
    fn group_hidden(&self, group: &MaybeRef<Object<'a>>) -> bool {
        group.as_obj_ref().is_some_and(|reference| {
            self.hidden
                .as_ref()
                .is_some_and(|hidden| hidden.contains(&id(reference)))
        })
    }

    /// Whether the visibility expression `expression` (`[/And …]`, `[/Or …]`,
    /// `[/Not …]` over groups) evaluates visible. An expression too deep or
    /// malformed to evaluate, anywhere in it, counts as visible as a whole:
    /// its content then stays in place and is extracted and redacted like
    /// any other.
    fn expression_visible(&self, expression: &MaybeRef<Object<'a>>, depth: usize) -> bool {
        let mut work = MAX_EXPRESSION_WORK;
        self.evaluate(expression, depth, &mut work).unwrap_or(true)
    }

    /// The visibility `expression` evaluates to, or `None` when it is too
    /// deep, too costly (past the `work` left) or malformed to evaluate. A
    /// dictionary operand is a group; any other dictionary is malformed.
    fn evaluate(
        &self,
        expression: &MaybeRef<Object<'a>>,
        depth: usize,
        work: &mut usize,
    ) -> Option<bool> {
        if depth > MAX_EXPRESSION_DEPTH {
            return None;
        }
        *work = work.checked_sub(1)?;
        let array = match (expression, self.pdf.resolve(expression)?) {
            (MaybeRef::Ref(_), Object::Dict(group)) if group.is_type(b"OCG") => {
                return Some(!self.group_hidden(expression));
            }
            (_, Object::Array(array)) => array,
            _ => return None,
        };
        let items: Vec<MaybeRef<Object<'a>>> = array.raw_iter().collect();
        let (MaybeRef::NotRef(Object::Name(operator)), operands) = items.split_first()? else {
            return None;
        };
        let visible = operands
            .iter()
            .map(|o| self.evaluate(o, depth + 1, work))
            .collect::<Option<Vec<bool>>>()?;
        match (&**operator, visible.as_slice()) {
            (b"And", [_, ..]) => Some(visible.iter().all(|&v| v)),
            (b"Or", [_, ..]) => Some(visible.iter().any(|&v| v)),
            (b"Not", [one]) => Some(!one),
            _ => None,
        }
    }
}

/// The layers hidden by the document's default configuration: every group
/// listed `/OFF`, or, when the base state is `/OFF`, every group not listed
/// `/ON`, each of an intent the configuration shares; `None` when the
/// configuration's `/Intent` is an empty array, which applies no rule.
fn hidden_groups(pdf: &Pdf) -> Option<HashSet<ObjectId>> {
    let Some(properties) = pdf
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get::<Dict<'_>>(b"OCProperties"))
    else {
        return Some(HashSet::new());
    };
    let Some(config) = properties.get::<Dict<'_>>(b"D") else {
        return Some(HashSet::new());
    };
    let ids = |key: &[u8], dict: &Dict<'_>| -> HashSet<ObjectId> {
        dict.get::<Array<'_>>(key)
            .map(|array| {
                array
                    .raw_iter()
                    .filter_map(|o| o.as_obj_ref().map(id))
                    .collect()
            })
            .unwrap_or_default()
    };
    let off = if config
        .name(b"BaseState")
        .is_some_and(|state| &*state == b"OFF")
    {
        let on = ids(b"ON", &config);
        ids(b"OCGs", &properties)
            .into_iter()
            .filter(|id| !on.contains(id))
            .collect()
    } else {
        ids(b"OFF", &config)
    };
    // A group whose intent the configuration does not share has no effect
    // on visibility (8.11.2.1), so it hides nothing.
    let viewing = intents(&config);
    if viewing.is_empty() {
        return None;
    }
    let all = |intents: &[Vec<u8>]| intents.iter().any(|intent| intent == b"All");
    Some(
        off.into_iter()
            .filter(|&id| {
                let group = pdf.object(id).and_then(Object::into_dict);
                let intended = group.map(|group| intents(&group)).unwrap_or_default();
                all(&viewing) || all(&intended) || intended.iter().any(|i| viewing.contains(i))
            })
            .collect(),
    )
}

/// The intents `dict` (a group or a configuration) names in `/Intent`: a
/// name or an array of names (empty when it is an empty array), `/View`
/// when it has none.
fn intents(dict: &Dict<'_>) -> Vec<Vec<u8>> {
    match dict.get::<Object<'_>>(b"Intent") {
        Some(Object::Name(name)) => vec![name.to_vec()],
        Some(Object::Array(names)) => names
            .iter::<Object<'_>>()
            .filter_map(|name| name.into_name().map(|name| name.to_vec()))
            .collect(),
        _ => vec![b"View".to_vec()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{held, read_id};
    use crate::pdf::object::{Dictionary, Id, Value, dict};
    use crate::pdf::reader::reference_to;
    use crate::pdf::writer::Document;

    /// A document whose default configuration turns `layer` off, with the
    /// group's and the configuration's intents as given.
    fn document(group: Dictionary, config: Dictionary) -> (Document, Id) {
        let mut doc = Document::new((1, 7));
        let layer = doc.add(group);
        let mut default = dict! { "OFF" => vec![layer.into()] };
        for (key, value) in config.iter() {
            default.set(key, value.clone());
        }
        let pages =
            doc.add(dict! { "Type" => "Pages", "Kids" => Vec::<Value>::new(), "Count" => 0 });
        let catalog = doc.add(dict! {
            "Type" => "Catalog",
            "Pages" => pages,
            "OCProperties" => dict! { "OCGs" => vec![layer.into()], "D" => default },
        });
        doc.set_root(catalog);
        (doc, layer)
    }

    /// Whether the optional content of `memberships`, each written as an
    /// object of `doc`, is hidden.
    fn hidden(mut doc: Document, memberships: Vec<Value>) -> Vec<bool> {
        let ids: Vec<Id> = memberships.into_iter().map(|m| doc.add(m)).collect();
        let pdf = held(doc, &ids);
        let layers = Layers::read(&pdf);
        ids.iter()
            .map(|&id| layers.is_hidden(&reference_to(read_id(id))))
            .collect()
    }

    /// An optional-content membership dictionary over `expression`.
    fn membership(expression: Value) -> Value {
        Value::Dict(dict! { "Type" => "OCMD", "VE" => expression })
    }

    /// A `/Not` over `operand`.
    fn not(operand: Value) -> Value {
        Value::Array(vec![Value::name("Not"), operand])
    }

    /// A malformed operand makes the whole expression count as visible: a
    /// `/Not` does not turn its fallback into hidden.
    #[test]
    fn a_malformed_expression_is_visible_as_a_whole() {
        let (doc, layer) = document(dict! { "Type" => "OCG" }, Dictionary::new());
        let found = hidden(
            doc,
            vec![
                membership(not(5.into())),
                membership(not(layer.into())),
                membership(not(not(layer.into()))),
            ],
        );
        assert_eq!(found, [false, false, true]);
    }

    /// A group meant only for design has no effect on viewing, so being off
    /// hides nothing; one meant for viewing does.
    #[test]
    fn a_group_of_another_intent_hides_nothing() {
        let group_hidden = |group: Dictionary, config: Dictionary| {
            let (doc, layer) = document(group, config);
            let pdf = held(doc, &[]);
            Layers::read(&pdf).is_hidden(&reference_to(read_id(layer)))
        };
        assert!(!group_hidden(
            dict! { "Type" => "OCG", "Intent" => "Design" },
            Dictionary::new(),
        ));
        assert!(group_hidden(
            dict! { "Type" => "OCG", "Intent" => vec!["Design".into(), "View".into()] },
            Dictionary::new(),
        ));
        assert!(group_hidden(
            dict! { "Type" => "OCG", "Intent" => "Design" },
            dict! { "Intent" => "All" },
        ));
        // An empty configuration intent applies no rule: nothing is hidden,
        // not even through an expression's negation.
        assert!(!group_hidden(
            dict! { "Type" => "OCG" },
            dict! { "Intent" => Vec::<Value>::new() },
        ));
        let (doc, layer) = document(
            dict! { "Type" => "OCG" },
            dict! { "Intent" => Vec::<Value>::new() },
        );
        let negated = Value::Dict(dict! { "Type" => "OCMD", "VE" => not(layer.into()) });
        assert_eq!(hidden(doc, vec![negated]), [false]);
    }

    /// An ordinary dictionary is no group, so an expression over one is
    /// malformed and visible; an expression sharing one subexpression many
    /// times over is bounded, and visible past the bound.
    #[test]
    fn malformed_and_costly_expressions_are_visible() {
        let (mut doc, layer) = document(dict! { "Type" => "OCG" }, Dictionary::new());
        let plain = doc.add(dict! { "Name" => "x" });
        let mut shared = doc.add(not(layer.into()));
        for _ in 0..30 {
            shared = doc.add(vec![Value::name("And"), shared.into(), shared.into()]);
        }
        let found = hidden(
            doc,
            vec![membership(not(plain.into())), membership(shared.into())],
        );
        assert_eq!(found, [false, false]);
    }
}
