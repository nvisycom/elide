//! HTML loader: parses HTML into the shared markup [`ExtractedItem`]
//! stream.
//!
//! Text nodes, every element attribute, and HTML comments are emitted as
//! items. Attribute values pass through verbatim (a `mailto:` URL has its
//! email matched in place). `<script>` / `<style>` bodies follow the
//! loader's [`script_policy`] / [`style_policy`].
//!
//! [`script_policy`]: HtmlLoader::script_policy
//! [`style_policy`]: HtmlLoader::style_policy

use std::collections::HashMap;

use ego_tree::{NodeId, NodeRef};
use elide_core::Result;
use elide_core::modality::Hint;
use elide_core::modality::text::{Text, TextData, TextLocation};
use scraper::Html;
use scraper::node::Node;

use super::html_handler::{
    ElementTarget, FORMAT_ID, HtmlAddress, HtmlEncoder, HtmlHandler, HtmlItem,
};
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::ExtractHandler;

/// How the loader handles `<script>` or `<style>` element bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptPolicy {
    /// Skip the element entirely; its body never enters the detection
    /// stream.
    #[default]
    Skip,
    /// Treat the element body as plain text and scan it like a regular
    /// text node.
    ScanText,
}

/// Loader for HTML files. Produces one [`HtmlHandler`] per input.
#[derive(Debug, Clone, Default)]
pub(crate) struct HtmlLoader {
    /// How `<script>` element bodies enter the detection stream.
    pub script_policy: ScriptPolicy,
    /// How `<style>` element bodies enter the detection stream.
    pub style_policy: ScriptPolicy,
}

impl Loader<Text> for HtmlLoader {
    type Handler = HtmlHandler;

    async fn decode(&self, content: ContentData) -> Result<HtmlHandler> {
        let text = content.decode()?;
        let dom = Html::parse_document(&text);
        let items = build_items(&dom, self);
        Ok(ExtractHandler::new(
            FORMAT_ID.clone(),
            HtmlEncoder { raw: text },
            items,
        ))
    }
}

fn build_items(dom: &Html, loader: &HtmlLoader) -> Vec<HtmlItem> {
    let mut items = Vec::new();
    let mut text_index: usize = 0;
    let mut comment_index: usize = 0;
    let mut element_index: usize = 0;

    // The engine assigns each item a cumulative-offset span (see
    // `compute_item_starts`): item `n` spans `[offset, offset + value.len())`
    // where `offset` is the summed length of all prior items. We mirror that
    // running offset here so we can record, per text node, the engine-space
    // span its value occupies — the coordinate a sibling hint resolves to.
    let mut offset: usize = 0;
    let mut text_node_spans: HashMap<NodeId, TextLocation> = HashMap::new();

    for node in dom.tree.nodes() {
        match node.value() {
            Node::Text(t) => {
                if !skip_text_under(node) {
                    let len = t.text.len();
                    text_node_spans.insert(node.id(), TextLocation::new(offset, offset + len));
                    items.push(HtmlItem {
                        address: HtmlAddress::TextNode { index: text_index },
                        value: t.text.to_string(),
                        // Filled in a second pass, once every text node's
                        // span is known (a sibling may sit after this node).
                        hints: Vec::new(),
                    });
                    offset += len;
                }
                text_index += 1;
            }
            Node::Comment(c) => {
                let value = c.comment.to_string();
                offset += value.len();
                items.push(HtmlItem {
                    address: HtmlAddress::Comment {
                        index: comment_index,
                    },
                    value,
                    hints: Vec::new(),
                });
                comment_index += 1;
            }
            Node::Element(e) => {
                let element_name = e.name.local.as_ref();

                // Every attribute on this element. Values pass through
                // verbatim: URLs like `mailto:alice@x.com` have the email
                // matched in place by the recognizer.
                for (qn, val) in &e.attrs {
                    let value = val.to_string();
                    offset += value.len();
                    items.push(HtmlItem {
                        address: HtmlAddress::Element {
                            element_index,
                            target: ElementTarget::Attribute {
                                attr_name: qn.local.as_ref().to_owned(),
                            },
                        },
                        value,
                        hints: Vec::new(),
                    });
                }

                // `<script>` / `<style>` body, when policy says ScanText.
                let policy = match element_name {
                    "script" => Some(loader.script_policy),
                    "style" => Some(loader.style_policy),
                    _ => None,
                };
                if let Some(ScriptPolicy::ScanText) = policy
                    && let Some(body) = first_child_text(node)
                {
                    offset += body.len();
                    items.push(HtmlItem {
                        address: HtmlAddress::Element {
                            element_index,
                            target: ElementTarget::Text,
                        },
                        value: body,
                        hints: Vec::new(),
                    });
                }

                element_index += 1;
            }
            _ => {}
        }
    }

    attach_sibling_hints(dom, &mut items, &text_node_spans);
    items
}

/// Second pass: give each text-node item one located hint per *other* text
/// node under its nearest block-level ancestor.
///
/// Surfaces the surrounding prose as out-of-band hints when a text node
/// sits inside an inline wrapper (`<code>4111…</code>`) that splits the
/// sentence into multiple chunks. Each sibling becomes its own
/// [`Hint<Text>`] located at the sibling's engine-space span — the same
/// coordinate the sibling's own chunk carries — so a boost can point back
/// at exactly which neighbour fired it. The node's own text is excluded so
/// a hint never echoes the chunk's own bytes.
fn attach_sibling_hints(dom: &Html, items: &mut [HtmlItem], spans: &HashMap<NodeId, TextLocation>) {
    // Walk text nodes in the same document order the items were pushed; the
    // i-th text-node item lines up with the i-th non-skipped text node.
    let mut item_idx = 0;
    for node in dom.tree.nodes() {
        let Node::Text(_) = node.value() else {
            continue;
        };
        if skip_text_under(node) {
            continue;
        }
        // Advance to the matching text-node item, skipping non-text items.
        while !matches!(items[item_idx].address, HtmlAddress::TextNode { .. }) {
            item_idx += 1;
        }
        items[item_idx].hints = sibling_text_hints(node, spans);
        item_idx += 1;
    }
}

/// One located hint per *other* text node under `text_node`'s nearest
/// block-level ancestor. Empty when there's no block ancestor or no other
/// text. The walk targets the nearest *block* ancestor; stopping at the
/// immediate inline parent would yield only the chunk's own text.
fn sibling_text_hints(
    text_node: NodeRef<'_, Node>,
    spans: &HashMap<NodeId, TextLocation>,
) -> Vec<Hint<Text>> {
    let Some(ancestor) = nearest_block_ancestor(text_node) else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    for descendant in ancestor.descendants() {
        if descendant.id() == text_node.id() {
            continue;
        }
        let Node::Text(t) = descendant.value() else {
            continue;
        };
        let trimmed = t.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        // A sibling with no recorded span was skipped (e.g. script/style
        // body); skip it as a hint too rather than fabricate a location.
        let Some(location) = spans.get(&descendant.id()) else {
            continue;
        };
        hints.push(Hint::new(
            location.clone(),
            TextData::new(trimmed.to_owned()),
        ));
    }
    hints
}

/// Walk parents until we hit a block-level element (or root).
fn nearest_block_ancestor(text_node: NodeRef<'_, Node>) -> Option<NodeRef<'_, Node>> {
    let mut current = text_node.parent();
    while let Some(node) = current {
        if let Node::Element(e) = node.value()
            && is_block_element(e.name.local.as_ref())
        {
            return Some(node);
        }
        current = node.parent();
    }
    None
}

fn is_block_element(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "li"
            | "td"
            | "th"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "dt"
            | "dd"
            | "section"
            | "article"
            | "aside"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "figcaption"
            | "caption"
    )
}

/// Don't emit text-node items for text directly inside a `<script>` or
/// `<style>` element; those bodies are handled by the script / style
/// policy on the parent. The `text_index` counter still advances so
/// encode's document-order index lines up with decode.
fn skip_text_under(text_node: NodeRef<'_, Node>) -> bool {
    text_node
        .parent()
        .and_then(|p| p.value().as_element())
        .map(|e| matches!(e.name.local.as_ref(), "script" | "style"))
        .unwrap_or(false)
}

fn first_child_text(node: NodeRef<'_, Node>) -> Option<String> {
    let child = node.first_child()?;
    match child.value() {
        Node::Text(t) => Some(t.text.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::DataWriter;
    use elide_core::modality::text::{TextLocation, TextReplacement};
    use elide_core::operator::Redactions;

    use super::*;
    use crate::Handler;

    async fn load(raw: &str) -> HtmlHandler {
        HtmlLoader::default()
            .decode(ContentData::from_text(raw))
            .await
            .expect("html decode succeeds")
    }

    fn encoded(h: &HtmlHandler) -> String {
        h.encode().unwrap().decode().unwrap()
    }

    #[tokio::test]
    async fn encode_unchanged_round_trips() {
        let raw = "<html><head></head><body><p>Hello</p></body></html>";
        let h = load(raw).await;
        assert_eq!(encoded(&h), raw);
    }

    #[tokio::test]
    async fn redact_replaces_text_node() {
        let raw = "<html><head></head><body><p>Hello</p><p>World</p></body></html>";
        let mut h = load(raw).await;
        let first = h.read_next().await.unwrap().unwrap();
        let mut rs = Redactions::new();
        rs.push(first.location, TextReplacement::substituted("[REDACTED]"));
        h.write_at(rs).await.unwrap();
        let out = encoded(&h);
        assert!(out.contains("[REDACTED]"));
        assert!(out.contains("World"));
    }

    #[tokio::test]
    async fn stream_yields_text_attribute_and_comment() {
        let raw = r#"<html><head></head><body><!-- secret 1 --><img alt="hello" title="alt"></body></html>"#;
        let mut h = load(raw).await;
        let mut values: Vec<String> = Vec::new();
        while let Some(chunk) = h.read_next().await.unwrap() {
            values.push(chunk.data.as_str().to_owned());
        }
        assert!(values.iter().any(|v| v == " secret 1 "));
        assert!(values.iter().any(|v| v == "hello"));
        assert!(values.iter().any(|v| v == "alt"));
    }

    #[tokio::test]
    async fn attribute_redact_round_trips() {
        let raw = r#"<html><head></head><body><img alt="alice@example.com"></body></html>"#;
        let mut h = load(raw).await;
        let mut loc = None;
        while let Some(chunk) = h.read_next().await.unwrap() {
            if chunk.data.as_str() == "alice@example.com" {
                loc = Some(chunk.location);
                break;
            }
        }
        let mut rs = Redactions::new();
        rs.push(
            loc.expect("alt attribute chunk"),
            TextReplacement::substituted("[email]"),
        );
        h.write_at(rs).await.unwrap();
        let out = encoded(&h);
        assert!(out.contains(r#"alt="[email]""#), "alt not rewritten: {out}");
    }

    #[tokio::test]
    async fn comment_redact_round_trips() {
        let raw = "<html><head></head><body><!-- alice@example.com --></body></html>";
        let mut h = load(raw).await;
        let mut loc = None;
        while let Some(chunk) = h.read_next().await.unwrap() {
            let s = chunk.data.as_str();
            if let Some(at) = s.find("alice") {
                loc = Some(TextLocation::new(
                    chunk.location.start + at,
                    chunk.location.start + at + "alice@example.com".len(),
                ));
                break;
            }
        }
        let mut rs = Redactions::new();
        rs.push(
            loc.expect("comment chunk"),
            TextReplacement::substituted("[email]"),
        );
        h.write_at(rs).await.unwrap();
        let out = encoded(&h);
        assert!(
            out.contains("<!-- [email] -->"),
            "comment not rewritten: {out}"
        );
    }

    #[tokio::test]
    async fn script_scan_text_policy_emits_body() {
        let raw =
            r#"<html><head></head><body><script>var a="alice@example.com";</script></body></html>"#;
        let loader = HtmlLoader {
            script_policy: ScriptPolicy::ScanText,
            ..HtmlLoader::default()
        };
        let mut h = loader
            .decode(ContentData::from_text(raw))
            .await
            .expect("decode");
        let mut found = false;
        while let Some(chunk) = h.read_next().await.unwrap() {
            if chunk.data.as_str().contains("alice@example.com") {
                found = true;
            }
        }
        assert!(found, "script body should be scanned under ScanText");
    }
}
