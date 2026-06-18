//! HTML loader: parses HTML into the shared markup [`RedactableItem`]
//! stream.
//!
//! Text nodes, every element attribute, and HTML comments are emitted as
//! items. Attribute values pass through verbatim (a `mailto:` URL has its
//! email matched in place). `<script>` / `<style>` bodies follow the
//! loader's [`script_policy`] / [`style_policy`].
//!
//! [`script_policy`]: HtmlLoader::script_policy
//! [`style_policy`]: HtmlLoader::style_policy

use ego_tree::NodeRef;
use scraper::Html;
use scraper::node::Node;
use elide_core::Error;
use elide_core::modality::text::Text;

use super::MarkupHandler;
use super::html_handler::{
    ElementTarget, FORMAT_ID, HtmlAddress, HtmlEncoder, HtmlHandler, HtmlItem,
};
use crate::Loader;
use crate::content::ContentData;

/// How the loader handles `<script>` or `<style>` element bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptPolicy {
    /// Skip the element entirely — its body never enters the detection
    /// stream.
    #[default]
    Skip,
    /// Treat the element body as plain text and scan it like a regular
    /// text node.
    ScanText,
}

/// Loader for HTML files. Produces one [`HtmlHandler`] per input.
#[derive(Debug, Clone, Default)]
pub struct HtmlLoader {
    /// How `<script>` element bodies enter the detection stream.
    pub script_policy: ScriptPolicy,
    /// How `<style>` element bodies enter the detection stream.
    pub style_policy: ScriptPolicy,
}

impl Loader<Text> for HtmlLoader {
    type Handler = HtmlHandler;

    async fn decode(&self, content: ContentData) -> Result<HtmlHandler, Error> {
        let text = content.decode()?;
        let dom = Html::parse_document(&text);
        let items = build_items(&dom, self);
        Ok(MarkupHandler::new(
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

    for node in dom.tree.nodes() {
        match node.value() {
            Node::Text(t) => {
                if !skip_text_under(node) {
                    let hints = sibling_text_hint(node, &t.text);
                    items.push(HtmlItem {
                        address: HtmlAddress::TextNode { index: text_index },
                        value: t.text.to_string(),
                        hints,
                    });
                }
                text_index += 1;
            }
            Node::Comment(c) => {
                items.push(HtmlItem {
                    address: HtmlAddress::Comment {
                        index: comment_index,
                    },
                    value: c.comment.to_string(),
                    hints: Vec::new(),
                });
                comment_index += 1;
            }
            Node::Element(e) => {
                let element_name = e.name.local.as_ref();

                // Every attribute on this element. Values pass through
                // verbatim — URLs like `mailto:alice@x.com` have the email
                // matched in place by the recognizer.
                for (qn, val) in &e.attrs {
                    items.push(HtmlItem {
                        address: HtmlAddress::Element {
                            element_index,
                            target: ElementTarget::Attribute {
                                attr_name: qn.local.as_ref().to_owned(),
                            },
                        },
                        value: val.to_string(),
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

    items
}

/// Collect the surrounding-text content of the text node's nearest
/// block-level ancestor as a single hint string.
///
/// Surfaces the surrounding sentence as an out-of-band hint when a text
/// node sits inside an inline wrapper (`<code>4111…</code>`) that splits
/// the prose into multiple chunks. The walk targets the nearest *block*
/// ancestor; stopping at the immediate inline parent would yield only the
/// chunk's own text. `own_text` is excluded so the hint doesn't echo the
/// node's own bytes.
fn sibling_text_hint(text_node: NodeRef<'_, Node>, own_text: &str) -> Vec<String> {
    let Some(ancestor) = nearest_block_ancestor(text_node) else {
        return Vec::new();
    };
    let mut buf = String::new();
    for descendant in ancestor.descendants() {
        if let Node::Text(t) = descendant.value() {
            let chunk = t.text.as_ref();
            if chunk == own_text {
                continue;
            }
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(chunk);
        }
    }
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        Vec::new()
    } else {
        vec![trimmed.to_owned()]
    }
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
/// `<style>` element — those bodies are handled by the script / style
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
    use elide_core::redaction::Redactions;

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
