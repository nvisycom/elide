//! Optional-content layers: which the document's default configuration hides.
//!
//! A layer (an optional-content group) the default configuration turns off is
//! content a reader does not see, so the rebuild cuts what it marks: each
//! marked-content sequence tagged with it (`/OC /name BDC … EMC`) and each
//! XObject that belongs to it. A visible layer's content stays, as plain page
//! content.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::pdf::reader::{DictionaryExt, DocumentExt};

/// How deep a visibility expression (`/VE`) may nest before it is refused.
const MAX_EXPRESSION_DEPTH: usize = 32;

/// How many operands one visibility expression is evaluated through, all
/// told, so a small expression sharing subexpressions does not expand into
/// a vast evaluation.
const MAX_EXPRESSION_WORK: usize = 4096;

/// The hidden layers of a document, and the reads that decide whether a piece
/// of optional content belongs to one.
pub(super) struct Layers<'a> {
    doc: &'a Document,
    /// The groups hidden, or `None` when the configuration applies no
    /// visibility rule (its `/Intent` is an empty array), so everything
    /// shows.
    hidden: Option<HashSet<ObjectId>>,
    /// Whether each membership held as an object of its own hides what it
    /// marks: content may name one any number of times, and it is evaluated
    /// once.
    memberships: RefCell<HashMap<ObjectId, bool>>,
    /// Whether each visibility expression held as an object of its own
    /// evaluates visible, as memberships may share one.
    expressions: RefCell<HashMap<ObjectId, Evaluated>>,
}

impl<'a> Layers<'a> {
    /// The layers of `doc`.
    pub(super) fn read(doc: &'a Document) -> Self {
        Self {
            doc,
            hidden: hidden_groups(doc),
            memberships: RefCell::new(HashMap::new()),
            expressions: RefCell::new(HashMap::new()),
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
        let Some(hidden) = &self.hidden else {
            return false;
        };
        if let Object::Reference(id) = membership
            && hidden.contains(id)
        {
            return true;
        }
        let Some(dict) = self.doc.resolve_dict(membership) else {
            return false;
        };
        let Ok(id) = membership.as_reference() else {
            return self.membership_hidden(dict);
        };
        if let Some(&hidden) = self.memberships.borrow().get(&id) {
            return hidden;
        }
        let hidden = self.membership_hidden(dict);
        self.memberships.borrow_mut().insert(id, hidden);
        hidden
    }

    /// Whether the membership dictionary `dict` hides what it marks.
    fn membership_hidden(&self, dict: &Dictionary) -> bool {
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
        group.as_reference().is_ok_and(|id| {
            self.hidden
                .as_ref()
                .is_some_and(|hidden| hidden.contains(&id))
        })
    }

    /// Whether the visibility expression `expression` (`[/And …]`, `[/Or …]`,
    /// `[/Not …]` over groups) evaluates visible. An expression too deep or
    /// malformed to evaluate, anywhere in it, counts as visible as a whole:
    /// its content then stays in place and is extracted and redacted like
    /// any other.
    fn expression_visible(&self, expression: &Object, depth: usize) -> bool {
        let mut work = MAX_EXPRESSION_WORK;
        self.evaluate(expression, depth, &mut work)
            .is_none_or(|evaluated| evaluated.visible)
    }

    /// What `expression` evaluates to, or `None` when it is too deep, too
    /// costly (past the `work` left) or malformed to evaluate. A dictionary
    /// operand is a group; any other dictionary is malformed.
    ///
    /// An expression held as an object of its own is evaluated once; naming
    /// it again costs what evaluating it did, and is as deep, so the limits
    /// hold as they would without the cache.
    fn evaluate(&self, expression: &Object, depth: usize, work: &mut usize) -> Option<Evaluated> {
        if depth > MAX_EXPRESSION_DEPTH {
            return None;
        }
        if let Object::Reference(id) = expression
            && let Some(&cached) = self.expressions.borrow().get(id)
        {
            if depth + cached.height > MAX_EXPRESSION_DEPTH {
                return None;
            }
            *work = work.checked_sub(cached.visits)?;
            return Some(cached);
        }
        let before = *work;
        *work = work.checked_sub(1)?;
        match expression {
            Object::Reference(id) => match self.doc.get_object(*id).ok()? {
                Object::Array(array) => {
                    let (visible, height) = self.operation(array, depth, work)?;
                    let evaluated = Evaluated {
                        visible,
                        height,
                        visits: before - *work,
                    };
                    self.expressions.borrow_mut().insert(*id, evaluated);
                    Some(evaluated)
                }
                Object::Dictionary(group) if group.is_type(self.doc, b"OCG") => Some(Evaluated {
                    visible: !self.group_hidden(expression),
                    height: 0,
                    visits: 1,
                }),
                _ => None,
            },
            Object::Array(array) => {
                let (visible, height) = self.operation(array, depth, work)?;
                Some(Evaluated {
                    visible,
                    height,
                    visits: before - *work,
                })
            }
            _ => None,
        }
    }

    /// The visibility the expression `expression` (an operator and its
    /// operands) evaluates to, and how many levels its operands nest below
    /// it, as [`evaluate`](Self::evaluate).
    fn operation(
        &self,
        expression: &[Object],
        depth: usize,
        work: &mut usize,
    ) -> Option<(bool, usize)> {
        let (Object::Name(operator), operands) = expression.split_first()? else {
            return None;
        };
        let operands = operands
            .iter()
            .map(|o| self.evaluate(o, depth + 1, work))
            .collect::<Option<Vec<Evaluated>>>()?;
        let height = 1 + operands.iter().map(|o| o.height).max().unwrap_or(0);
        let visible: Vec<bool> = operands.iter().map(|o| o.visible).collect();
        let visible = match (operator.as_slice(), visible.as_slice()) {
            (b"And", [_, ..]) => visible.iter().all(|&v| v),
            (b"Or", [_, ..]) => visible.iter().any(|&v| v),
            (b"Not", [one]) => !one,
            _ => return None,
        };
        Some((visible, height))
    }
}

/// What a visibility expression evaluated to: whether it is visible, how
/// many levels its operands nest below it, and how many operands evaluating
/// it visited, itself included.
#[derive(Debug, Clone, Copy)]
struct Evaluated {
    visible: bool,
    height: usize,
    visits: usize,
}

/// The layers hidden by the document's default configuration: every group
/// listed `/OFF`, or, when the base state is `/OFF`, every group not listed
/// `/ON`, each of an intent the configuration shares; `None` when the
/// configuration's `/Intent` is an empty array, which applies no rule.
fn hidden_groups(doc: &Document) -> Option<HashSet<ObjectId>> {
    let Some(properties) = doc
        .catalog()
        .ok()
        .and_then(|catalog| catalog.resolved_dict(doc, b"OCProperties"))
    else {
        return Some(HashSet::new());
    };
    let Some(config) = properties.resolved_dict(doc, b"D") else {
        return Some(HashSet::new());
    };
    let ids = |key: &[u8], dict: &Dictionary| -> HashSet<ObjectId> {
        dict.resolved_array(doc, key)
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
    if viewing.is_empty() {
        return None;
    }
    let all = |intents: &[Vec<u8>]| intents.iter().any(|intent| intent == b"All");
    Some(
        off.into_iter()
            .filter(|&id| {
                let group = doc.get_dictionary(id).ok();
                let intended = group.map(|group| intents(doc, group)).unwrap_or_default();
                all(&viewing) || all(&intended) || intended.iter().any(|i| viewing.contains(i))
            })
            .collect(),
    )
}

/// The intents `dict` (a group or a configuration) names in `/Intent`: a
/// name or an array of names (empty when it is an empty array), `/View`
/// when it has none.
fn intents(doc: &Document, dict: &Dictionary) -> Vec<Vec<u8>> {
    match dict.resolved(doc, b"Intent") {
        Some(Object::Name(name)) => vec![name.clone()],
        Some(Object::Array(names)) => names
            .iter()
            .filter_map(|name| doc.resolve(name)?.as_name().ok().map(<[u8]>::to_vec))
            .collect(),
        _ => vec![b"View".to_vec()],
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
        let not = |operand: Object| Object::Array(vec![Object::from("Not"), operand]);
        assert!(!layers.is_hidden(&membership(not(5.into()))));
        assert!(!layers.is_hidden(&membership(not(layer.into()))));
        assert!(layers.is_hidden(&membership(not(not(layer.into())))));
    }

    /// A membership and an expression held as objects of their own are
    /// evaluated once, however often content names them.
    #[test]
    fn a_membership_named_again_is_evaluated_once() {
        let (mut doc, layer) = document(dictionary! { "Type" => "OCG" }, Dictionary::new());
        let expression = doc.add_object(vec![Object::from("Not"), layer.into()]);
        let membership = doc.add_object(
            dictionary! { "Type" => "OCMD", "VE" => vec![Object::from("Not"), expression.into()] },
        );
        let layers = Layers::read(&doc);
        for _ in 0..3 {
            assert!(layers.is_hidden(&Object::Reference(membership)));
        }
        assert_eq!(layers.memberships.borrow().len(), 1);
        assert_eq!(layers.expressions.borrow().len(), 1);
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
        // An empty configuration intent applies no rule: nothing is hidden,
        // not even through an expression's negation.
        let (doc, layer) = document(
            dictionary! { "Type" => "OCG" },
            dictionary! { "Intent" => Vec::<Object>::new() },
        );
        let layers = Layers::read(&doc);
        assert!(!layers.is_hidden(&Object::Reference(layer)));
        let negated = Object::Dictionary(dictionary! {
            "Type" => "OCMD",
            "VE" => vec![Object::from("Not"), layer.into()],
        });
        assert!(!layers.is_hidden(&negated));
    }

    /// An ordinary dictionary is no group, so an expression over one is
    /// malformed and visible; an expression sharing one subexpression many
    /// times over is bounded, and visible past the bound.
    #[test]
    fn malformed_and_costly_expressions_are_visible() {
        let (mut doc, layer) = document(dictionary! { "Type" => "OCG" }, Dictionary::new());
        let plain = doc.add_object(dictionary! { "Name" => "x" });
        // Hidden were it evaluated whole: every operand is the hidden layer.
        let mut shared = layer;
        for _ in 0..30 {
            shared = doc.add_object(vec![Object::from("And"), shared.into(), shared.into()]);
        }
        let layers = Layers::read(&doc);
        let membership = |expression: Object| {
            Object::Dictionary(dictionary! { "Type" => "OCMD", "VE" => expression })
        };
        let not_plain = vec![Object::from("Not"), plain.into()];
        assert!(!layers.is_hidden(&membership(Object::Array(not_plain))));
        assert!(!layers.is_hidden(&membership(shared.into())));
    }

    /// A membership whose `/VE` is `expression`.
    fn membership(expression: Object) -> Object {
        Object::Dictionary(dictionary! { "Type" => "OCMD", "VE" => expression })
    }

    /// An expression evaluated once and named again deeper is held to the
    /// depth limit as it would be evaluated there.
    #[test]
    fn a_cached_expression_is_held_to_the_depth_limit() {
        let (mut doc, layer) = document(dictionary! { "Type" => "OCG" }, Dictionary::new());
        let and = |operand: Object| Object::Array(vec![Object::from("And"), operand]);
        let hidden = doc.add_object(and(layer.into()));
        let mut deep = Object::Reference(hidden);
        for _ in 0..MAX_EXPRESSION_DEPTH {
            deep = and(deep);
        }
        let layers = Layers::read(&doc);
        assert!(layers.is_hidden(&membership(hidden.into())));
        assert!(!layers.is_hidden(&membership(deep)), "past the depth limit");
    }

    /// An expression evaluated once and named again costs what evaluating it
    /// did, so the work limit holds as it would without the cache.
    #[test]
    fn a_cached_expression_is_charged_its_work() {
        let (mut doc, layer) = document(dictionary! { "Type" => "OCG" }, Dictionary::new());
        let and = |operand: Object| Object::Array(vec![Object::from("And"), operand]);
        let children: Vec<ObjectId> = (0..MAX_EXPRESSION_WORK / 2)
            .map(|_| doc.add_object(and(layer.into())))
            .collect();
        let mut wide = vec![Object::from("And")];
        wide.extend(children.iter().map(|&child| Object::Reference(child)));
        let layers = Layers::read(&doc);
        assert!(layers.is_hidden(&membership(children[0].into())));
        assert!(
            !layers.is_hidden(&membership(Object::Array(wide))),
            "past the work limit"
        );
    }
}
