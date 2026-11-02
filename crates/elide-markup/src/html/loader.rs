//! HTML loader: runs the XML markup engine over HTML with lenient parsing.
//!
//! HTML has no engine of its own. Its hidden content (scripts and other active
//! content, comments, `<meta>` fields) is cut out first by the
//! [`MarkupSanitizer`]; then the shared XML engine ([`build_items`]) reads the
//! rest leniently (tolerating void elements, stray end tags, and bare `&`) with
//! the two HTML vocabularies it takes as plain element-name lists: the block
//! elements that group sibling text for context hints, and the `<style>` bodies
//! not to scan. Everything downstream, streaming, redaction, byte-faithful
//! splice, is the XML handler.

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::handler::FORMAT_ID;
use crate::engine::config::MarkupConfig;
use crate::engine::parser::build_items;
use crate::engine::recombine::markup_document;
use crate::sanitize::{Dialect, MarkupSanitizer};

/// HTML block-level elements: each ends a passage, and its text children form
/// one sibling-hint group, so a label and a value in separate passages of one
/// block (`<td>Card number<p>4111…</p></td>`) still surface to a boost. Names are lowercased and
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

/// HTML elements that end a passage besides the block elements: text does not
/// read across the document structure, a form control, or embedded content, so
/// `<title>Jane</title>…<body>Roe` or two `<option>`s never join into one
/// value. Names are lowercased and sorted.
const PASSAGE_BOUNDS: &[&str] = &[
    "audio", "body", "button", "canvas", "details", "embed", "fieldset", "figure", "frame",
    "frameset", "head", "hr", "html", "iframe", "input", "legend", "menu", "meter", "noscript",
    "object", "optgroup", "option", "output", "picture", "progress", "select", "summary", "svg",
    "tbody", "template", "textarea", "tfoot", "thead", "title", "video",
];

/// Element bodies the engine does not scan: a `<style>` sheet is presentation,
/// not content. (Scripts never reach the engine; the sanitizer removes them.)
const SKIP_BODY_ELEMENTS: &[&str] = &["style"];

/// Loader for HTML files. Produces one leaf [`Document`] per input.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HtmlLoader;

#[async_trait::async_trait]
impl DocumentLoader for HtmlLoader {
    async fn decode(&self, content: ContentData, policy: HiddenContentPolicy) -> Result<Document> {
        let sanitizer = MarkupSanitizer::new(Dialect::Html, policy);
        let (text, stripped) = sanitizer.sanitize(content.decode()?)?;
        let config = MarkupConfig::lenient(BLOCK_ELEMENTS, SKIP_BODY_ELEMENTS)
            .with_passage_bounds(PASSAGE_BOUNDS);
        // `build_items` already reports a `MalformedInput` parse error.
        let items = build_items(&text, config)?;
        Ok(markup_document(FORMAT_ID.clone(), text, items, sanitizer).with_stripped(stripped))
    }
}

#[cfg(test)]
mod tests {
    use elide_codec::{Document, DocumentPart};
    use elide_core::modality::text::{Text, TextReplacement};
    use elide_core::modality::{DataWriter, StreamDataReader};
    use elide_core::redaction::{Disposition, Redactions};

    use super::*;

    /// The body stream part of a decoded HTML document, downcast to `Text`.
    fn body(doc: &mut Document) -> &mut elide_codec::TypedStream<Text> {
        match &mut doc.parts_mut()[0] {
            DocumentPart::Stream { handle, .. } => {
                handle.downcast_mut::<Text>().expect("text body")
            }
            _ => panic!("part 0 is the body stream"),
        }
    }

    async fn load_with(raw: &str, policy: HiddenContentPolicy) -> Document {
        HtmlLoader
            .decode(ContentData::from_text(raw), policy)
            .await
            .expect("html decode succeeds")
    }

    async fn load(raw: &str) -> Document {
        load_with(raw, HiddenContentPolicy::default()).await
    }

    fn encoded(doc: &Document) -> String {
        doc.encode().unwrap().decode().unwrap()
    }

    fn values(doc: &mut Document) -> Vec<String> {
        body(doc)
            .chunks()
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.data.as_str().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn text_does_not_read_across_document_structure_or_form_controls() {
        let raw = concat!(
            "<html><head><title>Jane</title></head><body>Roe ",
            "<select><option>4111</option><option>1111</option></select></body></html>",
        );
        let mut doc = load(raw).await;
        assert_eq!(values(&mut doc), ["Jane", "Roe ", "4111", "1111"]);
    }

    #[tokio::test]
    async fn html_named_references_read_decoded() {
        let raw = "<p>Jane&nbsp;Roe &amp; jane&#46;roe&#64;example.com</p>";
        let mut doc = load(raw).await;
        assert_eq!(values(&mut doc), ["Jane\u{a0}Roe & jane.roe@example.com"]);
        assert_eq!(encoded(&doc), raw);
    }

    #[tokio::test]
    async fn an_unquoted_attribute_value_is_read_and_redacted() {
        let raw = "<p><input value=jane@example.com disabled><a title=Jane/onclick=x>t</a></p>";
        let mut doc = load(raw).await;
        let vs = values(&mut doc);
        assert!(vs.iter().any(|v| v == "jane@example.com"), "{vs:?}");
        let chunk = body(&mut doc)
            .chunks()
            .unwrap()
            .into_iter()
            .find(|chunk| chunk.data.as_str() == "jane@example.com")
            .expect("chunk");
        let mut redactions = Redactions::new();
        redactions.push(chunk.location, TextReplacement::substituted("[EMAIL]"));
        body(&mut doc).write_at(redactions).await.unwrap();
        assert!(encoded(&doc).contains("<input value=[EMAIL] disabled>"));
    }

    #[tokio::test]
    async fn encode_unchanged_round_trips() {
        let raw = "<html><head></head><body><p>Hello</p></body></html>";
        let doc = load(raw).await;
        assert_eq!(encoded(&doc), raw);
    }

    #[tokio::test]
    async fn stream_yields_text_attribute_and_a_redacted_comment() {
        let raw = r#"<html><body><!-- secret 1 --><img alt="hello" title="alt"></body></html>"#;
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::TreatAsContent);
        let mut doc = load_with(raw, policy).await;
        let vs = values(&mut doc);
        assert!(vs.iter().any(|v| v == " secret 1 "), "comment: {vs:?}");
        assert!(vs.iter().any(|v| v == "hello"), "alt: {vs:?}");
        assert!(vs.iter().any(|v| v == "alt"), "title: {vs:?}");
    }

    #[tokio::test]
    async fn attribute_redact_round_trips() {
        let raw = r#"<html><body><img alt="alice@example.com"></body></html>"#;
        let mut doc = load(raw).await;
        let chunk = body(&mut doc)
            .chunks()
            .unwrap()
            .into_iter()
            .find(|c| c.data.as_str() == "alice@example.com")
            .expect("chunk");
        let mut rs = Redactions::new();
        rs.push(chunk.location, TextReplacement::substituted("[email]"));
        body(&mut doc).write_at(rs).await.unwrap();
        assert!(
            encoded(&doc).contains(r#"alt="[email]""#),
            "alt not rewritten: {}",
            encoded(&doc)
        );
    }

    #[tokio::test]
    async fn hidden_content_is_removed_at_decode() {
        let raw = r#"<html><body><!-- ask Jane --><p onclick="x()">Hi</p><script>var a="alice@example.com";</script></body></html>"#;
        let mut doc = load(raw).await;
        assert!(doc.changed_at_decode());
        let vs = values(&mut doc);
        assert!(
            !vs.iter()
                .any(|v| v.contains("alice@example.com") || v.contains("Jane")),
            "hidden content reached detection: {vs:?}"
        );
        assert_eq!(encoded(&doc), "<html><body><p>Hi</p></body></html>");
    }

    /// The hint texts of the chunk reading `text`.
    fn hint_texts(doc: &mut Document, text: &str) -> Vec<String> {
        let chunk = body(doc)
            .chunks()
            .unwrap()
            .into_iter()
            .find(|chunk| chunk.data.as_str() == text)
            .expect("chunk");
        chunk
            .hints
            .iter()
            .map(|hint| hint.data.as_str().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn a_sibling_hint_links_passages_of_one_block() {
        // The <td> (in the real BLOCK_ELEMENTS) holds the label and, in a
        // nested <p>, the number: two passages its sibling hints link.
        let raw = "<table><tr><td>Card number<p>4111 1111 1111 1111</p></td></tr></table>";
        let mut doc = load(raw).await;
        let hints = hint_texts(&mut doc, "4111 1111 1111 1111");
        assert!(hints.iter().any(|hint| hint == "Card number"), "{hints:?}");
    }

    #[tokio::test]
    async fn text_in_one_passage_gets_no_sibling_hint() {
        let raw = "<table><tr><td>Card <code>4111 1111 1111 1111</code> on file</td></tr></table>";
        let mut doc = load(raw).await;
        let hints = hint_texts(&mut doc, "Card 4111 1111 1111 1111 on file");
        assert!(!hints.iter().any(|hint| hint == "on file"), "{hints:?}");
    }
}
