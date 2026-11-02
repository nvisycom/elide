//! [`MarkupSanitizer`]: remove a markup document's hidden content before its
//! text is extracted.
//!
//! The sanitizer finds the byte ranges of hidden content in the raw source and
//! cuts them out, leaving every other byte as it was, so the extraction that
//! follows (and its byte-faithful splice) runs over exactly what will ship:
//!
//! - **Active content**, always (HTML): `<script>`, `<object>`, `<applet>`,
//!   `<embed>`, and frame elements (`<iframe>`, `<frame>`, `<frameset>`),
//!   `on*` event-handler attributes, URLs that run script (`javascript:`,
//!   `vbscript:`, `data:text/html`, read the way a browser reads them, with
//!   character references decoded and tabs, line breaks, and leading control
//!   characters dropped), `<meta>` redirects and cookies, and conditional
//!   comments.
//! - **External references**, always: what the page loads from a server when
//!   opened (an image, a style sheet, a `<base>`, a form target, inline style
//!   that imports or loads a URL) and a hyperlink's `ping`; the hyperlink a
//!   reader follows stays. In XML, an `xml-stylesheet` instruction and an
//!   XInclude.
//! - **Content never displayed**, always (HTML): a `<template>`, and an element
//!   marked `hidden` (one only hidden until found stays).
//! - **Collaboration**, under [`Disposition::StripNonEssential`]: comments, the
//!   author's notes a reader never sees.
//! - **Metadata**, under [`Disposition::StripNonEssential`] (HTML): `<meta>` fields,
//!   except the ones that render the page (its character set, a rendering
//!   `http-equiv` directive, the viewport).
//!
//! A document type that declares entities, or in XML names an external one, is
//! refused outright: its text is never extracted, so it would ship past
//! detection.

mod html;
mod url;

use std::ops::Range;

use elide_codec::StrippedContent;
use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};
use elide_core::{Error, Result};
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};

use self::html::{attributes, hidden_attributes};
use self::url::css_loads_remote;
use crate::engine::parser::{bom_len, end_name, start_name};
use crate::engine::tag::HtmlTagExt;

/// The XInclude namespace, whose `include` element pulls in another document.
const XINCLUDE: &str = "http://www.w3.org/2001/XInclude";

/// The markup dialect a [`MarkupSanitizer`] reads: strict XML, or lenient HTML
/// with its element vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// Well-formed XML, whose only hidden content is its comments and the
    /// references it makes outside the document.
    Xml,
    /// HTML, read leniently.
    Html,
}

/// Removes a markup document's hidden content under a [`HiddenContentPolicy`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct MarkupSanitizer {
    dialect: Dialect,
    policy: HiddenContentPolicy,
}

/// What the sanitizer does with an element.
enum Verdict {
    /// Keep the element; its attributes are checked on their own.
    Keep,
    /// Remove the element with everything inside it.
    Tree(HiddenContent),
    /// Remove the element's tag alone (a void or self-closing element).
    Tag(HiddenContent),
}

impl MarkupSanitizer {
    /// A sanitizer for `dialect` documents under `policy`.
    pub(crate) fn new(dialect: Dialect, policy: HiddenContentPolicy) -> Self {
        Self { dialect, policy }
    }

    /// `raw` with its hidden content cut out, and the record of what was.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
    /// markup cannot be read, or its document type declares entities or (in XML)
    /// names an external one.
    pub(crate) fn sanitize(&self, raw: String) -> Result<(String, StrippedContent)> {
        let removals = self.removals(&raw)?;
        let mut stripped = StrippedContent::new();
        if removals.is_empty() {
            return Ok((raw, stripped));
        }
        let mut out = String::with_capacity(raw.len());
        let mut cursor = 0;
        for removal in removals {
            out.push_str(&raw[cursor..removal.range.start]);
            cursor = removal.range.end;
            stripped.record(removal.content, None, 1);
        }
        out.push_str(&raw[cursor..]);
        Ok((out, stripped))
    }

    /// Check that `raw` carries no hidden content this sanitizer would remove.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) if hidden content survived, or the errors of
    /// [`sanitize`](Self::sanitize).
    pub(crate) fn check(&self, raw: &str) -> Result<()> {
        if self.removals(raw)?.is_empty() {
            Ok(())
        } else {
            Err(Error::sanitization(
                "encoded markup still carries hidden content",
            ))
        }
    }

    /// The disjoint byte ranges of `raw` to remove, in document order, each
    /// with the hidden content it holds.
    fn removals(&self, raw: &str) -> Result<Vec<Removal>> {
        let html = self.dialect == Dialect::Html;
        let mut reader = NsReader::from_str(raw);
        if html {
            let config = reader.config_mut();
            config.check_end_names = false;
            config.allow_unmatched_ends = true;
            config.allow_dangling_amp = true;
        }
        // quick-xml reports positions past a leading BOM, but the ranges index
        // the original bytes.
        let bom = bom_len(raw);
        let mut last = bom;
        let mut removals = Vec::new();
        let mut removing: Option<Region> = None;
        let mut style: Option<StyleSheet> = None;

        loop {
            let (namespace, event) = reader
                .read_resolved_event()
                .map_err(|e| Error::malformed(format!("malformed markup: {e}")))?;
            let in_xinclude =
                matches!(namespace, ResolveResult::Bound(Namespace(uri)) if uri == XINCLUDE);
            let span = last..reader.buffer_position() as usize + bom;
            last = span.end;

            if let Some(region) = removing.as_mut() {
                match &event {
                    Event::Start(e) if start_name(e, raw, html)? == region.name => {
                        region.depth += 1;
                    }
                    Event::End(e) if end_name(e, raw, html)? == region.name => {
                        region.depth -= 1;
                        if region.depth == 0 {
                            removals.push(region.removal(span.end));
                            removing = None;
                        }
                    }
                    // An element left open by lenient HTML runs to the end of
                    // the document.
                    Event::Eof => {
                        removals.push(region.removal(raw.len()));
                        break;
                    }
                    _ => {}
                }
                continue;
            }

            // A style sheet is decided once its text is read: one that loads
            // from a server goes whole.
            if let Some(sheet) = style.as_mut() {
                match &event {
                    Event::Text(text) => sheet.css.push_str(text.as_ref()),
                    Event::CData(text) => sheet.css.push_str(text.as_ref()),
                    Event::End(e) if end_name(e, raw, html)? == "style" => {
                        if css_loads_remote(&sheet.css) {
                            removals.push(Removal {
                                range: sheet.start..span.end,
                                content: HiddenContent::Link,
                            });
                        }
                        style = None;
                    }
                    Event::Eof => break,
                    _ => {}
                }
                continue;
            }

            // A construct a browser ends before quick-xml does hides live
            // markup inside it from this reader.
            if html && let Some(construct) = ends_early(&event) {
                return Err(Error::malformed(format!(
                    "a browser ends this HTML {construct} at an earlier `>`, so markup inside it renders"
                )));
            }
            match &event {
                Event::Eof => break,
                Event::DocType(doctype) => self.check_doctype(doctype.as_ref())?,
                Event::PI(instruction) if instruction.as_ref().starts_with("xml-stylesheet") => {
                    removals.push(Removal {
                        range: span,
                        content: HiddenContent::Link,
                    });
                }
                // A conditional comment is markup an old browser runs, whatever
                // the policy does with comments.
                Event::Comment(text) if is_conditional(text.as_ref()) => {
                    removals.push(Removal {
                        range: span,
                        content: HiddenContent::Script,
                    });
                }
                Event::Comment(_)
                    if self.policy.collaboration == Disposition::StripNonEssential =>
                {
                    removals.push(Removal {
                        range: span,
                        content: HiddenContent::Comment,
                    });
                }
                Event::Start(e) | Event::Empty(e) => {
                    let is_start = matches!(event, Event::Start(_));
                    // An HTML tag is read once, as a browser reads it.
                    let tag = if html { Some(e.html5_tag(raw)?) } else { None };
                    let attributes = tag.as_ref().map(|tag| attributes(raw, tag)).transpose()?;
                    let name = match &tag {
                        Some(tag) => tag.name.to_ascii_lowercase(),
                        None => start_name(e, raw, false)?,
                    };
                    let verdict = if let Some(attributes) = &attributes {
                        self.html_verdict(attributes, &name, is_start)
                    } else if in_xinclude && name == "include" {
                        Verdict::Tree(HiddenContent::Link)
                    } else {
                        Verdict::Keep
                    };
                    match verdict {
                        Verdict::Tree(content) if is_start => {
                            removing = Some(Region {
                                name,
                                content,
                                depth: 1,
                                start: span.start,
                            });
                        }
                        Verdict::Tree(content) | Verdict::Tag(content) => {
                            removals.push(Removal {
                                range: span,
                                content,
                            });
                        }
                        Verdict::Keep => {
                            if is_start && html && name == "style" {
                                style = Some(StyleSheet {
                                    start: span.start,
                                    css: String::new(),
                                });
                            }
                            if let Some(attributes) = attributes {
                                removals.extend(hidden_attributes(attributes, &name));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(removals)
    }

    /// Refuse a document type whose text detection would not see: one that
    /// declares entities, or (in XML, where a consumer fetches it) names an
    /// external one. HTML's document type is never fetched.
    fn check_doctype(&self, doctype: &str) -> Result<()> {
        if doctype.contains("<!ENTITY") {
            return Err(Error::malformed(
                "markup document type declares entities, whose text would bypass detection",
            ));
        }
        let external = doctype.contains("SYSTEM") || doctype.contains("PUBLIC");
        if self.dialect == Dialect::Xml && external {
            return Err(Error::malformed(
                "XML document type names an external one, whose text would bypass detection",
            ));
        }
        Ok(())
    }
}

/// An element being removed with its subtree: its lowercased name, the hidden
/// content it is, the open depth of that same name (so a nested one resolves
/// to the matching close), and the byte where its start tag begins.
struct Region {
    name: String,
    content: HiddenContent,
    depth: usize,
    start: usize,
}

impl Region {
    /// The removal of this region, closed at byte `end`.
    fn removal(&self, end: usize) -> Removal {
        Removal {
            range: self.start..end,
            content: self.content,
        }
    }
}

/// A `<style>` element being read: where it starts, and its text so far.
struct StyleSheet {
    start: usize,
    css: String,
}

/// A byte range of the source to remove, with the hidden content it holds.
struct Removal {
    range: Range<usize>,
    content: HiddenContent,
}

/// The construct `event` is, when a browser ends it before quick-xml does, so
/// markup quick-xml reads as inside it renders: a comment a browser closes
/// abruptly (`<!-->`, `<!--->`) or at `--!>`, and a processing instruction,
/// CDATA section, or document type with a `>` inside, which a browser ends at
/// its first `>`.
fn ends_early(event: &Event<'_>) -> Option<&'static str> {
    match event {
        Event::Comment(text) => {
            let text: &str = text.as_ref();
            (text.starts_with('>') || text.starts_with("->") || text.contains("--!>"))
                .then_some("comment")
        }
        Event::PI(text) => text
            .as_ref()
            .contains('>')
            .then_some("processing instruction"),
        Event::CData(text) => text.as_ref().contains('>').then_some("CDATA section"),
        Event::DocType(text) => text.as_ref().contains('>').then_some("document type"),
        _ => None,
    }
}

/// Whether a comment's `text` is a conditional comment (`[if IE]>…`), markup
/// an old browser parses and runs.
fn is_conditional(text: &str) -> bool {
    text.trim_start().starts_with("[if")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(policy: HiddenContentPolicy) -> MarkupSanitizer {
        MarkupSanitizer::new(Dialect::Html, policy)
    }

    fn sanitize(sanitizer: MarkupSanitizer, raw: &str) -> String {
        sanitizer.sanitize(raw.to_owned()).expect("sanitize").0
    }

    #[test]
    fn removes_scripts_plugins_and_event_handlers() {
        let raw = r#"<p onclick="track('jane')">Hi</p><script>var u = {"email":"jane@example.com"};</script><embed src="x.swf"><object data="y"><param name="a">fallback</object><a href=" JavaScript:steal()">x</a><a href="/ok">y</a>"#;
        let out = sanitize(html(HiddenContentPolicy::default()), raw);
        assert_eq!(out, r#"<p>Hi</p><a>x</a><a href="/ok">y</a>"#);
    }

    #[test]
    fn a_handler_after_a_slash_or_unquoted_is_removed() {
        // A browser reads a `/` between attributes like whitespace, so each of
        // these is an `a` with an `onclick` handler.
        let raw = r#"<a/onclick=steal()>x</a><a title="t"/onclick=steal()>y</a><a href=javascript:steal()>z</a>"#;
        let out = sanitize(html(HiddenContentPolicy::default()), raw);
        assert_eq!(out, r#"<a>x</a><a title="t">y</a><a>z</a>"#);
        // An attribute removed from between two others never joins them.
        let raw = r#"<img src="https://x.test/p.png"alt="Jane">"#;
        let out = sanitize(html(HiddenContentPolicy::default()), raw);
        assert_eq!(out, r#"<img alt="Jane">"#);
    }

    #[test]
    fn a_script_url_spelled_with_html_references_is_removed() {
        // A browser decodes HTML's named references and numeric ones without
        // a `;` before it reads the scheme.
        let raw = r#"<a href="javascript&colon;steal()">x</a><a href="jav&Tab;ascript:steal()">y</a><a href="javascript&#58steal()">z</a>"#;
        let out = sanitize(html(HiddenContentPolicy::default()), raw);
        assert_eq!(out, "<a>x</a><a>y</a><a>z</a>");
    }

    #[test]
    fn a_construct_a_browser_ends_early_is_refused() {
        // A browser reads each `<img>` here as an element, quick-xml as the
        // text of a comment, processing instruction, CDATA section, or
        // document type.
        for raw in [
            "<p><!--><img src=x onerror=steal()>--></p>",
            "<p><!---><img src=x onerror=steal()>--></p>",
            "<p><!-- a --!><img src=x onerror=steal()>--></p>",
            "<p><?x><img src=x onerror=steal()>?></p>",
            "<p><![CDATA[><img src=x onerror=steal()>]]></p>",
            "<!DOCTYPE html [<!-- x -->]><p>t</p>",
        ] {
            let sanitizer = html(
                HiddenContentPolicy::default().with_collaboration(Disposition::TreatAsContent),
            );
            let error = sanitizer.sanitize(raw.to_owned()).expect_err(raw);
            assert!(error.to_string().contains("earlier `>`"), "{raw}: {error}");
        }
        // An ordinary comment, processing instruction, and document type stay.
        let raw = "<!DOCTYPE html><?xml version=\"1.0\"?><p><!-- note --></p>";
        let sanitizer =
            html(HiddenContentPolicy::default().with_collaboration(Disposition::TreatAsContent));
        assert!(sanitizer.sanitize(raw.to_owned()).is_ok());
    }

    #[test]
    fn an_unclosed_script_runs_to_the_end() {
        let out = sanitize(
            html(HiddenContentPolicy::default()),
            "<p>a</p><script>var x = 1;",
        );
        assert_eq!(out, "<p>a</p>");
    }

    #[test]
    fn comments_follow_the_collaboration_action() {
        let raw = "<p>a<!-- ask Jane --></p>";
        assert_eq!(
            sanitize(html(HiddenContentPolicy::default()), raw),
            "<p>a</p>"
        );
        let redact = HiddenContentPolicy::default().with_collaboration(Disposition::TreatAsContent);
        assert_eq!(sanitize(html(redact), raw), raw);
    }

    #[test]
    fn meta_fields_follow_the_metadata_disposition_but_render_tags_stay() {
        let raw = r#"<head><meta charset="utf-8"><meta name="viewport" content="w"><meta name="author" content="Jane Roe"><meta http-equiv="content-type" content="text/html"></head>"#;
        let out = sanitize(html(HiddenContentPolicy::default()), raw);
        assert_eq!(
            out,
            r#"<head><meta charset="utf-8"><meta name="viewport" content="w"><meta http-equiv="content-type" content="text/html"></head>"#
        );
        let redact = HiddenContentPolicy::default().with_metadata(Disposition::TreatAsContent);
        assert_eq!(sanitize(html(redact), raw), raw);
    }

    /// A redirect or a cookie acts when the page opens, so it goes whatever the
    /// metadata disposition, and a `javascript:` redirect with it.
    #[test]
    fn acting_meta_directives_go_under_every_policy() {
        let raw = r#"<head><meta http-equiv="refresh" content="0;url=javascript:steal()"><meta http-equiv="Set-Cookie" content="id=1"></head>"#;
        let redact = HiddenContentPolicy::default().with_metadata(Disposition::TreatAsContent);
        assert_eq!(sanitize(html(redact), raw), "<head></head>");
    }

    /// A browser decodes character references and drops tabs, line breaks, and
    /// leading control characters before it reads a URL's scheme, so the check
    /// does too.
    #[test]
    fn an_encoded_or_split_script_url_is_still_caught() {
        let policy = HiddenContentPolicy::default();
        for href in [
            "&#106;avascript:alert(1)",
            "java&#x09;script:alert(1)",
            "java\tscript:alert(1)",
            "&#x01; javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html,<script>alert(1)</script>",
        ] {
            let raw = format!(r#"<a href="{href}">x</a>"#);
            assert_eq!(sanitize(html(policy), &raw), "<a>x</a>", "{href}");
        }
    }

    #[test]
    fn frames_go_with_their_inline_or_remote_page() {
        let raw = r#"<p>a</p><iframe srcdoc="<script>s()</script>"></iframe><iframe src="https://x"/><frameset><frame src="b.html"></frameset>"#;
        assert_eq!(
            sanitize(html(HiddenContentPolicy::default()), raw),
            "<p>a</p>"
        );
    }

    /// What the page loads from a server when opened goes; a hyperlink a
    /// reader follows, and a resource next to the page, stay.
    #[test]
    fn remote_loads_go_and_hyperlinks_stay() {
        let raw = r#"<img src="https://t/pixel.gif" alt="logo"><img src="logo.png"><img srcset="a.png 1x, https://t/b.png 2x"><a href="https://site" ping="https://t/ping">site</a><link rel="stylesheet" href="//cdn/x.css"><link rel="stylesheet" href="local.css"><base href="https://t/"><div style="background:url('https://t/bg')">d</div><style>@import "https://t/x.css";</style><style>p{color:red}</style>"#;
        assert_eq!(
            sanitize(html(HiddenContentPolicy::default()), raw),
            r#"<img alt="logo"><img src="logo.png"><img><a href="https://site">site</a><link rel="stylesheet" href="local.css"><div>d</div><style>p{color:red}</style>"#
        );
    }

    #[test]
    fn undisplayed_elements_go_and_findable_ones_stay() {
        let raw = r#"<template><p>Jane</p></template><div hidden><p>Roe</p></div><img hidden src="a.png"><p hidden="until-found">found</p>"#;
        assert_eq!(
            sanitize(html(HiddenContentPolicy::default()), raw),
            r#"<p hidden="until-found">found</p>"#
        );
    }

    /// A conditional comment is markup an old browser runs, so it goes even
    /// when the policy keeps comments.
    #[test]
    fn a_conditional_comment_goes_under_every_policy() {
        let raw = "<!--[if IE]><script>s()</script><![endif]--><!-- kept note -->";
        let redact = HiddenContentPolicy::default().with_collaboration(Disposition::TreatAsContent);
        assert_eq!(sanitize(html(redact), raw), "<!-- kept note -->");
    }

    #[test]
    fn xml_drops_its_external_references_and_refuses_an_external_dtd() {
        let xml = MarkupSanitizer::new(Dialect::Xml, HiddenContentPolicy::default());
        let raw = r#"<?xml version="1.0"?><?xml-stylesheet href="https://t/s.xsl"?><doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="https://t/part.xml"/><p>a</p></doc>"#;
        assert_eq!(
            sanitize(xml, raw),
            r#"<?xml version="1.0"?><doc xmlns:xi="http://www.w3.org/2001/XInclude"><p>a</p></doc>"#
        );
        let external = r#"<!DOCTYPE doc SYSTEM "https://t/doc.dtd"><doc/>"#;
        assert!(xml.sanitize(external.to_owned()).is_err());
        // HTML's document type is never fetched, so it stays.
        let xhtml = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd"><p>a</p>"#;
        assert_eq!(sanitize(html(HiddenContentPolicy::default()), xhtml), xhtml);
    }

    #[test]
    fn the_record_names_each_removal_by_what_it_is() {
        let raw = r#"<head><meta name="author" content="Jane"></head><!-- note --><p onclick="x()">a</p><script>s()</script><object>o</object>"#;
        let (_, stripped) = html(HiddenContentPolicy::default())
            .sanitize(raw.to_owned())
            .expect("sanitize");
        let counts: Vec<(HiddenContent, usize)> = stripped
            .iter()
            .map(|entry| (entry.content, entry.count))
            .collect();
        assert_eq!(
            counts,
            [
                (HiddenContent::Properties, 1),
                (HiddenContent::Comment, 1),
                (HiddenContent::Script, 2),
                (HiddenContent::Control, 1),
            ]
        );
    }

    #[test]
    fn xml_removes_only_comments() {
        let raw = r#"<doc onload="x"><!-- note --><script>kept</script></doc>"#;
        let xml = MarkupSanitizer::new(Dialect::Xml, HiddenContentPolicy::default());
        assert_eq!(
            sanitize(xml, raw),
            r#"<doc onload="x"><script>kept</script></doc>"#
        );
    }

    #[test]
    fn a_document_type_declaring_entities_is_refused() {
        let raw = r#"<!DOCTYPE d [<!ENTITY who "Jane Roe">]><d>&who;</d>"#;
        let xml = MarkupSanitizer::new(Dialect::Xml, HiddenContentPolicy::default());
        assert!(xml.sanitize(raw.to_owned()).is_err());
    }

    #[test]
    fn check_rejects_surviving_hidden_content() {
        let sanitizer = html(HiddenContentPolicy::default());
        assert!(sanitizer.check("<p>clean</p>").is_ok());
        assert!(sanitizer.check("<p>a<!-- x --></p>").is_err());
    }
}
