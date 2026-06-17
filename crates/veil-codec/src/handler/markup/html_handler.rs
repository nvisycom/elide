//! HTML handler side: the [`HtmlHandler`] type, its [`Format`]
//! descriptor, and the [`HtmlEncoder`] that re-serializes a mutated
//! [`RedactableItem`] stream back into HTML.
//!
//! Streaming, reading, and redaction live on the shared
//! [`MarkupHandler`](super::MarkupHandler); this side supplies only the
//! DOM re-serializer (the `scraper` parser lives in the loader). The
//! [`EncodePlan`] buckets the item stream by kind so a single DOM walk
//! resolves each ordinal in O(1).

use ego_tree::NodeId;
use scraper::Html;
use scraper::node::Node;
use veil_core::Error;
use veil_core::modality::text::Text;

use super::{ElementTarget, MarkupEncoder, MarkupHandler, RedactableItem, RedactableKind};
use crate::content::ContentData;
use crate::{Format, FormatId};

/// Stable [`FormatId`] for the HTML codec.
pub const FORMAT_ID: FormatId = FormatId::from_static("veil.text.html");

/// Handler type for loaded HTML content.
pub type HtmlHandler = MarkupHandler<HtmlEncoder>;

/// [`Format`] descriptor registered into [`crate::CodecRegistry`].
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), super::HtmlLoader::default())
        .with_extensions(["html", "htm"])
        .with_content_types(["text/html"])
}

/// Re-serializes a mutated item stream by splicing values back into a
/// fresh parse of the retained source.
#[derive(Debug)]
pub struct HtmlEncoder {
    pub(super) raw: String,
}

impl MarkupEncoder for HtmlEncoder {
    fn encode(&self, items: &[RedactableItem]) -> Result<ContentData, Error> {
        let mut dom = Html::parse_document(&self.raw);
        EncodePlan::from_items(items).apply(&mut dom);
        Ok(ContentData::new(dom.html().into_bytes().into()))
    }
}

/// Pre-bucketed view of the items, indexed by kind so encode's single DOM
/// walk resolves each ordinal in O(1).
struct EncodePlan<'a> {
    /// Indexed by text-node ordinal in document order. `Some(value)` when
    /// the loader emitted an item for that text node; `None` for skipped
    /// text nodes (e.g. script / style children under a `Skip` policy).
    text_values: Vec<Option<&'a str>>,
    /// Indexed by comment ordinal. Same shape as `text_values`.
    comment_values: Vec<Option<&'a str>>,
    /// Indexed by element ordinal. Each entry is the (possibly empty) list
    /// of element-bound items at that element.
    element_targets: Vec<Vec<ElementTargetPatch<'a>>>,
}

/// A single element-bound patch staged for encode.
struct ElementTargetPatch<'a> {
    target: &'a ElementTarget,
    value: &'a str,
}

impl<'a> EncodePlan<'a> {
    fn from_items(items: &'a [RedactableItem]) -> Self {
        let mut text_values: Vec<Option<&'a str>> = Vec::new();
        let mut comment_values: Vec<Option<&'a str>> = Vec::new();
        let mut element_targets: Vec<Vec<ElementTargetPatch<'a>>> = Vec::new();

        for item in items {
            match &item.kind {
                RedactableKind::TextNode { index } => {
                    grow_to(&mut text_values, *index);
                    text_values[*index] = Some(item.value.as_str());
                }
                RedactableKind::Comment { index } => {
                    grow_to(&mut comment_values, *index);
                    comment_values[*index] = Some(item.value.as_str());
                }
                RedactableKind::Element {
                    element_index,
                    target,
                } => {
                    while element_targets.len() <= *element_index {
                        element_targets.push(Vec::new());
                    }
                    element_targets[*element_index].push(ElementTargetPatch {
                        target,
                        value: item.value.as_str(),
                    });
                }
            }
        }

        Self {
            text_values,
            comment_values,
            element_targets,
        }
    }

    /// Walk `dom` in document order, applying the per-kind patches against
    /// the matching ordinals. Mutates `dom` in place.
    fn apply(&self, dom: &mut Html) {
        let mut text_seen = 0usize;
        let mut comment_seen = 0usize;
        let mut element_seen = 0usize;

        let node_ids: Vec<_> = dom.tree.nodes().map(|n| n.id()).collect();
        for node_id in node_ids {
            let Some(node) = dom.tree.get(node_id) else {
                continue;
            };
            match node.value() {
                Node::Text(_) => {
                    if let Some(value) = self.text_values.get(text_seen).copied().flatten() {
                        write_text(dom, node_id, value);
                    }
                    text_seen += 1;
                }
                Node::Comment(_) => {
                    if let Some(value) = self.comment_values.get(comment_seen).copied().flatten() {
                        write_comment(dom, node_id, value);
                    }
                    comment_seen += 1;
                }
                Node::Element(_) => {
                    if let Some(targets) = self.element_targets.get(element_seen) {
                        for et in targets {
                            apply_element_target(dom, node_id, et);
                        }
                    }
                    element_seen += 1;
                }
                _ => {}
            }
        }
    }
}

fn grow_to(v: &mut Vec<Option<&str>>, index: usize) {
    while v.len() <= index {
        v.push(None);
    }
}

fn write_text(dom: &mut Html, node_id: NodeId, value: &str) {
    if let Some(mut n) = dom.tree.get_mut(node_id)
        && let Node::Text(t) = n.value()
        && t.text.as_ref() != value
    {
        t.text = value.into();
    }
}

fn write_comment(dom: &mut Html, node_id: NodeId, value: &str) {
    if let Some(mut n) = dom.tree.get_mut(node_id)
        && let Node::Comment(c) = n.value()
        && c.comment.as_ref() != value
    {
        c.comment = value.into();
    }
}

fn apply_element_target(dom: &mut Html, node_id: NodeId, patch: &ElementTargetPatch<'_>) {
    match patch.target {
        ElementTarget::Attribute { attr_name } => {
            write_attribute(dom, node_id, attr_name, patch.value);
        }
        ElementTarget::Text => {
            write_text_child(dom, node_id, patch.value);
        }
    }
}

fn write_attribute(dom: &mut Html, element_id: NodeId, attr_name: &str, value: &str) {
    if let Some(mut n) = dom.tree.get_mut(element_id)
        && let Node::Element(e) = n.value()
    {
        for (qn, val) in &mut e.attrs {
            if qn.local.as_ref() == attr_name {
                if val.as_ref() != value {
                    *val = value.into();
                }
                return;
            }
        }
    }
}

fn write_text_child(dom: &mut Html, element_id: NodeId, value: &str) {
    let child_id = dom
        .tree
        .get(element_id)
        .and_then(|n| n.first_child().map(|c| c.id()));
    let Some(child_id) = child_id else {
        return;
    };
    write_text(dom, child_id, value);
}
