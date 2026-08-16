//! HTML loader: runs the XML markup engine over HTML with lenient parsing.
//!
//! HTML has no engine of its own. It configures the shared XML engine
//! ([`build_items`]) leniently (tolerating void elements, stray end tags, and
//! bare `&`) and supplies the two HTML vocabularies the engine takes as plain
//! element-name lists: the block elements that group sibling text for context
//! hints, and — translated from the loader's [`ScriptPolicy`] — the
//! `<script>` / `<style>` bodies not to scan. Everything downstream —
//! streaming, redaction, byte-faithful splice — is the XML handler.

use elide_core::Result;
use elide_core::modality::text::Text;

use super::config::MarkupConfig;
use super::html_handler::{FORMAT_ID, HtmlHandler, ScriptPolicy};
use super::xml_handler::XmlEncoder;
use super::xml_loader::build_items;
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::ExtractHandler;

/// HTML block-level elements: their text children form one sibling-hint group,
/// so prose split across inline wrappers (`Card <code>4111…</code> on file`)
/// still surfaces the surrounding context to a boost. Names are lowercased and
/// sorted. Membership is a linear scan (see `MarkupConfig::is_block`), fine for
/// a list this size checked once per closing tag.
const BLOCK_ELEMENTS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "caption",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

/// Loader for HTML files. Produces one [`HtmlHandler`] per input.
#[derive(Debug, Clone, Default)]
pub(crate) struct HtmlLoader {
    /// How `<script>` element bodies enter the detection stream.
    pub script_policy: ScriptPolicy,
    /// How `<style>` element bodies enter the detection stream.
    pub style_policy: ScriptPolicy,
}

impl HtmlLoader {
    /// The element bodies the engine should not scan: `<script>` / `<style>`
    /// under a [`ScriptPolicy::Skip`]. A [`ScriptPolicy::ScanText`] element is
    /// omitted, so its body is scanned as ordinary text.
    fn skip_body_elements(&self) -> Vec<&'static str> {
        let mut skip = Vec::new();
        if self.script_policy == ScriptPolicy::Skip {
            skip.push("script");
        }
        if self.style_policy == ScriptPolicy::Skip {
            skip.push("style");
        }
        skip
    }
}

#[async_trait::async_trait]
impl Loader<Text> for HtmlLoader {
    type Handler = HtmlHandler;

    async fn decode(&self, content: ContentData) -> Result<HtmlHandler> {
        let text = content.decode()?;
        let skip = self.skip_body_elements();
        let config = MarkupConfig::lenient(BLOCK_ELEMENTS, &skip);
        // `build_items` already reports a `MalformedInput` parse error.
        let items = build_items(&text, config)?;
        Ok(ExtractHandler::new(
            FORMAT_ID.clone(),
            XmlEncoder { raw: text },
            items,
        ))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::DataWriter;
    use elide_core::modality::text::TextReplacement;
    use elide_core::operator::Redactions;

    use super::*;
    use crate::Handler;

    async fn load_with(raw: &str, loader: HtmlLoader) -> HtmlHandler {
        loader
            .decode(ContentData::from_text(raw))
            .await
            .expect("html decode succeeds")
    }

    async fn load(raw: &str) -> HtmlHandler {
        load_with(raw, HtmlLoader::default()).await
    }

    fn encoded(h: &HtmlHandler) -> String {
        h.encode().unwrap().decode().unwrap()
    }

    async fn values(h: &mut HtmlHandler) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(chunk) = h.read_next().await.unwrap() {
            out.push(chunk.data.as_str().to_owned());
        }
        out
    }

    #[tokio::test]
    async fn encode_unchanged_round_trips() {
        let raw = "<html><head></head><body><p>Hello</p></body></html>";
        let h = load(raw).await;
        assert_eq!(encoded(&h), raw);
    }

    #[tokio::test]
    async fn stream_yields_text_attribute_and_comment() {
        let raw = r#"<html><body><!-- secret 1 --><img alt="hello" title="alt"></body></html>"#;
        let mut h = load(raw).await;
        let vs = values(&mut h).await;
        assert!(vs.iter().any(|v| v == " secret 1 "), "comment: {vs:?}");
        assert!(vs.iter().any(|v| v == "hello"), "alt: {vs:?}");
        assert!(vs.iter().any(|v| v == "alt"), "title: {vs:?}");
    }

    #[tokio::test]
    async fn attribute_redact_round_trips() {
        let raw = r#"<html><body><img alt="alice@example.com"></body></html>"#;
        let mut h = load(raw).await;
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str() == "alice@example.com" {
                break c;
            }
        };
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[email]"));
        h.write_at(rs).await.unwrap();
        assert!(
            encoded(&h).contains(r#"alt="[email]""#),
            "alt not rewritten: {}",
            encoded(&h)
        );
    }

    #[tokio::test]
    async fn script_body_skipped_by_default_scanned_on_request() {
        let raw = r#"<html><body><script>var a="alice@example.com";</script></body></html>"#;

        // Default: script bodies are skipped.
        let mut default = load(raw).await;
        let vs = values(&mut default).await;
        assert!(
            !vs.iter().any(|v| v.contains("alice@example.com")),
            "default leaked script body: {vs:?}"
        );

        // ScanText: the body is scanned.
        let loader = HtmlLoader {
            script_policy: ScriptPolicy::ScanText,
            ..HtmlLoader::default()
        };
        let mut scan = load_with(raw, loader).await;
        let vs = values(&mut scan).await;
        assert!(
            vs.iter().any(|v| v.contains("alice@example.com")),
            "scan missed script body: {vs:?}"
        );
    }

    #[tokio::test]
    async fn sibling_hints_span_the_real_block_vocabulary() {
        // A <td> (in the real BLOCK_ELEMENTS) groups its split text for hints.
        let raw = "<table><tr><td>Card <code>4111 1111 1111 1111</code> on file</td></tr></table>";
        let mut h = load(raw).await;
        let mut any_hint = false;
        while let Some(chunk) = h.read_next().await.unwrap() {
            if !chunk.hints.is_empty() {
                any_hint = true;
            }
        }
        assert!(any_hint, "expected sibling hints under the <td> block");
    }
}
