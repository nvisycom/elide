//! Strip the hidden content a PDF's content streams carry: the content of
//! optional-content layers hidden by default, and the alternate text marked
//! content attaches.
//!
//! Optional content lets a page carry content a reader does not see: a layer
//! (an optional-content group) that the document's default configuration turns
//! off. Such content is hidden content like any other, so it is cut from every
//! content stream before extraction: each marked-content sequence tagged with a
//! hidden layer (`/OC /name BDC … EMC`), and each XObject invocation (`Do`)
//! whose XObject belongs to one. Content of a visible layer is left in place
//! and extracted as page content; the layer configuration itself is removed by
//! the sanitizer afterwards.
//!
//! Marked content can also carry text of its own in an inline property list
//! (`/Span <</ActualText (…)>> BDC`): the text a reader's assistive technology
//! speaks in place of the glyphs, alternate descriptions, and abbreviation
//! expansions. None of it is drawn or extracted, so it is stripped too.

use std::collections::HashSet;

use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId};

use super::ext::SanitizeOperationExt;
use crate::document::content::{apply, drop_xobjects, plan};
use crate::document::object::DocumentExt;

/// How deep a visibility expression (`/VE`) may nest before it is refused.
const MAX_EXPRESSION_DEPTH: usize = 32;

/// What a pass over the content streams cut.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct ContentCuts {
    /// Marked-content sequences and XObject invocations of a hidden layer.
    pub(super) layers: usize,
    /// Property lists stripped of their alternate text.
    pub(super) alternate_text: usize,
}

impl ContentCuts {
    fn add(&mut self, other: Self) {
        self.layers += other.layers;
        self.alternate_text += other.alternate_text;
    }

    fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// Strip the hidden content of `doc`'s pages and form XObjects: cut the content
/// of every layer hidden by default, and the alternate text of every inline
/// marked-content property list. An XObject that belongs to a hidden layer is
/// dropped from every resource dictionary too, so the next prune removes it
/// from the file rather than leaving it unreferenced by the page but still
/// shipped.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a content stream cannot be decoded or
/// encoded.
pub(super) fn strip(doc: &mut Document) -> Result<ContentCuts> {
    let hidden = hidden_groups(doc);
    let layers = Layers {
        doc,
        hidden: &hidden,
    };

    let edits = plan(doc, |bytes, resources| layers.filter(bytes, resources))?;
    let hidden_xobjects: HashSet<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(&id, object)| match object {
            Object::Stream(stream) => stream.dict.get(b"OC").ok().map(|oc| (id, oc)),
            _ => None,
        })
        .filter(|(_, membership)| layers.is_hidden(membership, 0))
        .map(|(id, _)| id)
        .collect();

    let mut cuts = ContentCuts::default();
    for cut in apply(doc, edits)? {
        cuts.add(cut);
    }
    drop_xobjects(doc, &hidden_xobjects);
    Ok(cuts)
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
            .and_then(Object::as_array)
            .map(|array| array.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default()
    };
    let base_off = config
        .get(b"BaseState")
        .and_then(Object::as_name)
        .is_ok_and(|state| state == b"OFF");
    if base_off {
        let on = ids(b"ON", config);
        ids(b"OCGs", properties)
            .into_iter()
            .filter(|id| !on.contains(id))
            .collect()
    } else {
        ids(b"OFF", config)
    }
}

/// The hidden layers of a document, and the reads that decide whether a piece
/// of optional content belongs to one.
struct Layers<'a> {
    doc: &'a Document,
    hidden: &'a HashSet<ObjectId>,
}

impl Layers<'_> {
    /// `bytes` with every hidden marked-content sequence and hidden XObject
    /// invocation cut and every inline property list's alternate text
    /// stripped, re-encoded, and what was cut; `None` when nothing was.
    fn filter(
        &self,
        bytes: &[u8],
        resources: &[Dictionary],
    ) -> Result<Option<(Vec<u8>, ContentCuts)>> {
        let content =
            Content::decode(bytes).map_err(|e| Error::malformed(format!("decode content: {e}")))?;
        let mut kept: Vec<Operation> = Vec::with_capacity(content.operations.len());
        // One entry per open marked-content sequence: whether it hides.
        let mut open: Vec<bool> = Vec::new();
        let mut cuts = ContentCuts::default();
        for mut operation in content.operations {
            let inside_hidden = open.contains(&true);
            match operation.operator.as_str() {
                "BDC" | "BMC" => {
                    let hides = !inside_hidden
                        && operation.operator == "BDC"
                        && self.marks_hidden(&operation, resources);
                    cuts.layers += usize::from(hides);
                    open.push(hides);
                    if inside_hidden || hides {
                        continue;
                    }
                    cuts.alternate_text += usize::from(operation.strip_alternate_text());
                }
                "EMC" => {
                    open.pop();
                    if inside_hidden {
                        continue;
                    }
                }
                _ if inside_hidden => continue,
                "Do" if self.invokes_hidden(&operation, resources) => {
                    cuts.layers += 1;
                    continue;
                }
                _ => {}
            }
            kept.push(operation);
        }
        if cuts.is_empty() {
            return Ok(None);
        }
        let encoded = Content { operations: kept }
            .encode()
            .map_err(|e| Error::malformed(format!("encode content: {e}")))?;
        Ok(Some((encoded, cuts)))
    }

    /// Whether the `BDC` `operation` opens optional content of a hidden layer:
    /// `/OC` with a property-list name (or an inline dictionary) that resolves
    /// to one.
    fn marks_hidden(&self, operation: &Operation, resources: &[Dictionary]) -> bool {
        let [Object::Name(tag), properties] = operation.operands.as_slice() else {
            return false;
        };
        if tag != b"OC" {
            return false;
        }
        let membership = match properties {
            Object::Name(name) => self.lookup(resources, b"Properties", name),
            other => Some(other.clone()),
        };
        membership.is_some_and(|m| self.is_hidden(&m, 0))
    }

    /// Whether the `Do` `operation` invokes an XObject of a hidden layer.
    fn invokes_hidden(&self, operation: &Operation, resources: &[Dictionary]) -> bool {
        let [Object::Name(name)] = operation.operands.as_slice() else {
            return false;
        };
        self.lookup(resources, b"XObject", name)
            .and_then(|xobject| match xobject {
                Object::Reference(id) => self.doc.get_object(id).ok().cloned(),
                other => Some(other),
            })
            .and_then(|xobject| match xobject {
                Object::Stream(stream) => stream.dict.get(b"OC").ok().cloned(),
                _ => None,
            })
            .is_some_and(|membership| self.is_hidden(&membership, 0))
    }

    /// Whether the optional-content `membership` (a group, or a membership
    /// dictionary over groups) is hidden by the default configuration.
    fn is_hidden(&self, membership: &Object, depth: usize) -> bool {
        if let Object::Reference(id) = membership
            && self.hidden.contains(id)
        {
            return true;
        }
        let Some(dict) = self.doc.resolve_dict(membership) else {
            return false;
        };
        let is_membership = dict
            .get(b"Type")
            .and_then(Object::as_name)
            .is_ok_and(|kind| kind == b"OCMD");
        if !is_membership {
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
        let policy = dict
            .get(b"P")
            .and_then(Object::as_name)
            .unwrap_or(b"AnyOn".as_slice());
        match policy {
            b"AllOn" => groups.iter().any(|&hidden| hidden),
            b"AnyOff" => groups.iter().all(|&hidden| !hidden),
            b"AllOff" => groups.iter().any(|&hidden| !hidden),
            _ => groups.iter().all(|&hidden| hidden),
        }
    }

    /// The entry `name` of the `category` sub-dictionary (`/Properties`,
    /// `/XObject`, inline or referenced) of the first resource dictionary that
    /// has it.
    fn lookup(&self, resources: &[Dictionary], category: &[u8], name: &[u8]) -> Option<Object> {
        resources.iter().find_map(|dict| {
            let entries = dict
                .get(category)
                .ok()
                .and_then(|entries| self.doc.resolve_dict(entries))?;
            entries.get(name).ok().cloned()
        })
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

#[cfg(test)]
mod tests {
    use elide_core::redaction::HiddenContent;
    use lopdf::{Stream, dictionary};

    use super::*;
    use crate::document::sanitize::SanitizeDocumentExt;

    /// A one-page document with a visible layer `A` and a layer `B` the default
    /// configuration turns off (or, with `base_off`, a base state of `/OFF`
    /// that turns on only `A`). The page shows `visible` in `A`, `secret` in
    /// `B`, and invokes an image XObject that belongs to `B`. Returns the
    /// document and its two groups.
    fn document(base_off: bool) -> (Document, ObjectId, ObjectId) {
        let mut doc = Document::with_version("1.7");
        let a =
            doc.add_object(dictionary! { "Type" => "OCG", "Name" => Object::string_literal("A") });
        let b =
            doc.add_object(dictionary! { "Type" => "OCG", "Name" => Object::string_literal("B") });
        let image = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Image", "OC" => b },
            b"pixels".to_vec(),
        ));
        let content =
            b"/OC /MC0 BDC BT (visible) Tj ET EMC /OC /MC1 BDC BT (secret) Tj ET EMC /Im0 Do"
                .to_vec();
        let contents = doc.add_object(Stream::new(dictionary! {}, content));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "Contents" => contents,
            "Resources" => dictionary! {
                "Properties" => dictionary! { "MC0" => a, "MC1" => b },
                "XObject" => dictionary! { "Im0" => image },
            },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let config = if base_off {
            dictionary! { "BaseState" => "OFF", "ON" => vec![a.into()] }
        } else {
            dictionary! { "OFF" => vec![b.into()] }
        };
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages,
            "OCProperties" => dictionary! { "OCGs" => vec![a.into(), b.into()], "D" => config },
        });
        doc.trailer.set("Root", catalog);
        (doc, a, b)
    }

    fn page_content(doc: &Document) -> String {
        let page = doc.get_pages().into_values().next().unwrap();
        String::from_utf8_lossy(&doc.get_page_content(page)).into_owned()
    }

    #[test]
    fn a_hidden_layers_content_and_xobjects_are_cut() {
        for base_off in [false, true] {
            let (mut doc, ..) = document(base_off);
            assert_eq!(strip(&mut doc).unwrap().layers, 2);
            let content = page_content(&doc);
            assert!(content.contains("visible"), "{content}");
            assert!(!content.contains("secret"), "{content}");
            assert!(!content.contains("Im0"), "{content}");
        }
    }

    #[test]
    fn a_membership_dictionary_follows_its_policy() {
        let (mut doc, a, b) = document(false);
        let any_on =
            doc.add_object(dictionary! { "Type" => "OCMD", "OCGs" => vec![a.into(), b.into()] });
        let all_on = doc.add_object(dictionary! {
            "Type" => "OCMD", "OCGs" => vec![a.into(), b.into()], "P" => "AllOn",
        });
        let hidden = hidden_groups(&doc);
        let layers = Layers {
            doc: &doc,
            hidden: &hidden,
        };
        assert!(!layers.is_hidden(&Object::Reference(any_on), 0));
        assert!(layers.is_hidden(&Object::Reference(all_on), 0));
        let not_a = Object::Array(vec![Object::Name(b"Not".to_vec()), Object::Reference(a)]);
        assert!(!layers.expression_visible(&not_a, 0));
    }

    #[test]
    fn sanitizing_cuts_hidden_content_before_dropping_the_configuration() {
        let (mut doc, ..) = document(false);
        let stripped = doc.sanitize().unwrap();
        let layers: usize = stripped
            .iter()
            .filter(|entry| entry.content == HiddenContent::Layer)
            .map(|entry| entry.count)
            .sum();
        // Two cuts, the catalog's configuration, and the page's two property
        // lists naming the groups; the hidden image is pruned with its layer,
        // not counted again.
        assert_eq!(layers, 5);
        assert!(
            doc.objects.values().all(|object| match object {
                Object::Stream(stream) => stream.content != b"pixels",
                _ => true,
            }),
            "the hidden layer's image shipped"
        );
        assert!(doc.catalog().unwrap().get(b"OCProperties").is_err());
        let content = page_content(&doc);
        assert!(content.contains("visible") && !content.contains("secret"));
    }

    #[test]
    fn a_document_without_hidden_layers_is_untouched() {
        let (mut doc, ..) = document(false);
        doc.catalog_mut().unwrap().remove(b"OCProperties");
        assert_eq!(strip(&mut doc).unwrap().layers, 0);
        assert!(page_content(&doc).contains("secret"));
    }
}
