//! HTML loader: runs the shared markup engine over HTML with lenient parsing.
//!
//! HTML has no engine of its own. The [`MarkupSanitizer`] first rebuilds the
//! document from what renders, leaving out scripts and other active content,
//! anything else it does not name, and, under the policy, comments and
//! `<meta>` fields; then the markup engine ([`decode`]) reads the rest
//! leniently (tolerating void elements, stray end tags, and bare `&`) with the
//! three HTML vocabularies it takes as plain element-name lists: the block
//! elements that group sibling text for context hints, the `<style>` bodies
//! not to scan, and the other elements text does not read across. Everything
//! downstream, streaming, redaction, byte-faithful splice, is the markup
//! engine's, as for XML.
//!
//! [`MarkupSanitizer`]: crate::sanitize::MarkupSanitizer

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::handler::FORMAT_ID;
use crate::engine::config::MarkupConfig;
use crate::engine::document::decode;

/// HTML block-level elements: each ends a passage, and its text children form
/// one sibling-hint group, so a label and a value in separate passages of one
/// block (`<td>Card number<p>4111…</p></td>`) still surface to a boost. Names
/// are lowercased and sorted. Membership is a linear scan (see
/// `MarkupConfig::is_block`), fine for a list this size checked once per start
/// and end tag.
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
        let config = MarkupConfig::html(BLOCK_ELEMENTS, SKIP_BODY_ELEMENTS, PASSAGE_BOUNDS);
        decode(FORMAT_ID, config, content, policy)
    }
}

#[cfg(test)]
mod tests {
    use elide_codec::{Document, TypedStream};
    use elide_core::entity::builtins;
    use elide_core::modality::text::{Text, TextReplacement};
    use elide_core::modality::{Chunk, ChunkReader, DataReader, DataWriter};
    use elide_core::redaction::{Disposition, HiddenContentKind, Redactions};

    use super::*;
    use crate::engine::test_util::hint_texts;

    /// The body stream of a decoded HTML document, as `Text`.
    fn body(doc: &mut Document) -> TypedStream<'_, Text> {
        let (_, stream) = doc.streams_mut().into_iter().next().expect("a body");
        stream.downcast::<Text>().expect("text body")
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

    /// The body chunk reading `text`.
    fn chunk(doc: &mut Document, text: &str) -> Chunk<Text> {
        body(doc)
            .chunks()
            .unwrap()
            .into_iter()
            .find(|chunk| chunk.data.as_str() == text)
            .expect("chunk")
    }

    /// Redact the body chunk reading `text` whole with `replacement`.
    async fn redact(doc: &mut Document, text: &str, replacement: &str) {
        let location = chunk(doc, text).location;
        body(doc)
            .write_at(Redactions::from([(
                location,
                TextReplacement::replaced(replacement),
            )]))
            .await
            .unwrap();
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
        redact(&mut doc, "jane@example.com", "[EMAIL]").await;
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
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        let mut doc = load_with(raw, policy).await;
        let vs = values(&mut doc);
        assert!(vs.iter().any(|v| v == " secret 1 "), "comment: {vs:?}");
        assert!(vs.iter().any(|v| v == "hello"), "alt: {vs:?}");
        assert!(vs.iter().any(|v| v == "alt"), "title: {vs:?}");
    }

    #[tokio::test]
    async fn a_comment_is_flagged_and_visible_text_is_not() {
        let raw = "<html><body><!-- ask Jane --><p>Hi</p></body></html>";
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::FlagEverything);
        let mut doc = load_with(raw, policy).await;
        let stream = body(&mut doc);
        let flagged = stream.flagged().unwrap();
        assert_eq!(flagged.len(), 1, "{flagged:?}");
        let text = stream.read_at(&flagged[0].location).await.unwrap();
        assert_eq!(text.as_str(), " ask Jane ");
        assert_eq!(flagged[0].kind, HiddenContentKind::Collaboration);
    }

    #[tokio::test]
    async fn a_meta_field_is_flagged_and_the_viewport_is_not() {
        let raw = r#"<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta name="Author" content="Jane Roe"><meta name="theme-color" content="red"></head><body><p>Hi</p></body></html>"#;
        let policy = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
        let mut doc = load_with(raw, policy).await;
        let stream = body(&mut doc);
        let flagged = stream.flagged().unwrap();
        let mut fields = Vec::new();
        for item in &flagged {
            assert_eq!(item.kind, HiddenContentKind::Metadata);
            let text = stream.read_at(&item.location).await.unwrap();
            fields.push((text.as_str().to_owned(), item.label.clone()));
        }
        assert_eq!(
            fields,
            [
                ("Jane Roe".to_owned(), builtins::PERSON_NAME.to_ref()),
                ("red".to_owned(), builtins::UNRESOLVED.to_ref()),
            ]
        );
    }

    #[tokio::test]
    async fn a_meta_name_is_read_as_a_browser_reads_it() {
        let raw = r#"<html><head><meta name="view&#112;ort" content="width=device-width"><meta name="&#97;uthor" content="Jane Roe"></head><body><p>Hi</p></body></html>"#;
        let policy = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
        let mut doc = load_with(raw, policy).await;
        let stream = body(&mut doc);
        let flagged = stream.flagged().unwrap();
        assert_eq!(flagged.len(), 1, "{flagged:?}");
        assert_eq!(flagged[0].label, builtins::PERSON_NAME.to_ref());
        let text = stream.read_at(&flagged[0].location).await.unwrap();
        assert_eq!(text.as_str(), "Jane Roe");
    }

    #[tokio::test]
    async fn attribute_redact_round_trips() {
        let raw = r#"<html><body><img alt="alice@example.com"></body></html>"#;
        let mut doc = load(raw).await;
        redact(&mut doc, "alice@example.com", "[email]").await;
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

    #[tokio::test]
    async fn a_sibling_hint_links_passages_of_one_block() {
        // The <td> (in the real BLOCK_ELEMENTS) holds the label and, in a
        // nested <p>, the number: two passages its sibling hints link.
        let raw = "<table><tr><td>Card number<p>4111 1111 1111 1111</p></td></tr></table>";
        let mut doc = load(raw).await;
        let hints = hint_texts(&chunk(&mut doc, "4111 1111 1111 1111"));
        assert!(hints.iter().any(|hint| hint == "Card number"), "{hints:?}");
    }

    #[tokio::test]
    async fn text_in_one_passage_gets_no_sibling_hint() {
        let raw = "<table><tr><td>Card <code>4111 1111 1111 1111</code> on file</td></tr></table>";
        let mut doc = load(raw).await;
        let hints = hint_texts(&chunk(&mut doc, "Card 4111 1111 1111 1111 on file"));
        assert!(!hints.iter().any(|hint| hint == "on file"), "{hints:?}");
    }
}
