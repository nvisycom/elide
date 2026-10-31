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
//! - **Collaboration**, under [`Disposition::Strip`]: comments, the
//!   author's notes a reader never sees.
//! - **Metadata**, under [`Disposition::Strip`] (HTML): `<meta>` fields,
//!   except the ones that render the page (its character set, a rendering
//!   `http-equiv` directive, the viewport).
//!
//! A document type that declares entities, or in XML names an external one, is
//! refused outright: its text is never extracted, so it would ship past
//! detection.

use std::borrow::Cow;
use std::ops::Range;

use elide_codec::StrippedContent;
use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};
use elide_core::{Error, ErrorKind, Result};
use quick_xml::NsReader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};

use super::markup_parser::{bom_len, end_name, local_name, malformed};

/// HTML elements that run or embed active content, removed with their whole
/// subtree: a script, a plugin object, and a frame that loads another page or
/// carries one inline (`srcdoc`).
const ACTIVE_ELEMENTS: [&str; 6] = ["script", "object", "applet", "iframe", "frame", "frameset"];

/// HTML void elements: they have no end tag, so removing one removes its tag.
const VOID_ELEMENTS: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// URL schemes that run script when the URL is followed.
const SCRIPT_SCHEMES: [&str; 3] = ["javascript:", "vbscript:", "data:text/html"];

/// URL schemes (and the scheme-relative form) that reach a server.
const REMOTE_SCHEMES: [&str; 5] = ["http:", "https:", "ftp:", "//", "\\\\"];

/// The attributes whose value is a URL the page loads when it is opened. A
/// hyperlink's `href` is not among them: a link a reader follows is visible
/// content.
const LOADING_ATTRIBUTES: [&str; 10] = [
    "src",
    "srcset",
    "action",
    "formaction",
    "background",
    "poster",
    "data",
    "longdesc",
    "manifest",
    "icon",
];

/// The `http-equiv` directives that only affect how the page renders.
const RENDER_DIRECTIVES: [&str; 5] = [
    "content-type",
    "content-language",
    "x-ua-compatible",
    "content-security-policy",
    "default-style",
];

/// The `http-equiv` directives that act when the page opens: a redirect, and a
/// cookie set on the reader's browser.
const ACTING_DIRECTIVES: [&str; 2] = ["refresh", "set-cookie"];

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
    /// [`ErrorKind::MalformedInput`] if the markup cannot be read, or its
    /// document type declares entities or (in XML) names an external one.
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
    /// [`ErrorKind::Sanitization`] if hidden content survived, or the errors of
    /// [`sanitize`](Self::sanitize).
    pub(crate) fn check(&self, raw: &str) -> Result<()> {
        if self.removals(raw)?.is_empty() {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Sanitization,
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
            let (namespace, event) = reader.read_resolved_event().map_err(malformed)?;
            let in_xinclude =
                matches!(namespace, ResolveResult::Bound(Namespace(uri)) if uri == XINCLUDE);
            let span = last..reader.buffer_position() as usize + bom;
            last = span.end;

            if let Some(region) = removing.as_mut() {
                match &event {
                    Event::Start(e) if local_name(e) == region.name => region.depth += 1,
                    Event::End(e) if end_name(e) == region.name => {
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
                    Event::End(e) if end_name(e) == "style" => {
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
                Event::Comment(_) if self.policy.collaboration == Disposition::Strip => {
                    removals.push(Removal {
                        range: span,
                        content: HiddenContent::Comment,
                    });
                }
                Event::Start(e) | Event::Empty(e) => {
                    let name = local_name(e);
                    let is_start = matches!(event, Event::Start(_));
                    let verdict = if html {
                        self.html_verdict(raw, e, &name, is_start)
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
                        Verdict::Keep if html => {
                            if is_start && name == "style" {
                                style = Some(StyleSheet {
                                    start: span.start,
                                    css: String::new(),
                                });
                            }
                            removals.extend(hidden_attributes(raw, e, &name));
                        }
                        Verdict::Keep => {}
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
            return Err(Error::new(
                ErrorKind::MalformedInput,
                "markup document type declares entities, whose text would bypass detection",
            ));
        }
        let external = doctype.contains("SYSTEM") || doctype.contains("PUBLIC");
        if self.dialect == Dialect::Xml && external {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                "XML document type names an external one, whose text would bypass detection",
            ));
        }
        Ok(())
    }

    /// What happens to the HTML element `e` named `name`.
    fn html_verdict(&self, raw: &str, e: &BytesStart<'_>, name: &str, is_start: bool) -> Verdict {
        let attributes = attributes(raw, e);
        let attribute = |key: &str| {
            attributes
                .iter()
                .find(|attr| attr.key.eq_ignore_ascii_case(key))
                .map(|attr| attr.value)
        };
        let removal = |content| {
            if is_start && !VOID_ELEMENTS.contains(&name) {
                Verdict::Tree(content)
            } else {
                Verdict::Tag(content)
            }
        };
        if ACTIVE_ELEMENTS.contains(&name) {
            let content = if name == "script" {
                HiddenContent::Script
            } else {
                HiddenContent::Control
            };
            return removal(content);
        }
        match name {
            "embed" => return Verdict::Tag(HiddenContent::Control),
            "template" => return removal(HiddenContent::Undisplayed),
            "base" => return Verdict::Tag(HiddenContent::Link),
            "link" if attribute("href").is_some_and(is_remote) => {
                return Verdict::Tag(HiddenContent::Link);
            }
            "meta" => {
                if let Some(content) = self.meta_removal(&attributes) {
                    return Verdict::Tag(content);
                }
            }
            _ => {}
        }
        let hidden =
            attribute("hidden").is_some_and(|value| !value.eq_ignore_ascii_case("until-found"));
        if hidden {
            return removal(HiddenContent::Undisplayed);
        }
        Verdict::Keep
    }

    /// What a `<meta>` tag with `attributes` is removed as, or `None` when it
    /// stays: a directive that acts on opening goes whatever the policy, one
    /// that renders the page stays, and any other field is metadata.
    fn meta_removal(&self, attributes: &[Attribute<'_>]) -> Option<HiddenContent> {
        let attribute = |key: &str| {
            attributes
                .iter()
                .find(|attr| attr.key.eq_ignore_ascii_case(key))
                .map(|attr| attr.value.trim().to_ascii_lowercase())
        };
        if let Some(directive) = attribute("http-equiv") {
            if ACTING_DIRECTIVES.contains(&directive.as_str()) {
                return Some(HiddenContent::Action);
            }
            if RENDER_DIRECTIVES.contains(&directive.as_str()) {
                return None;
            }
        }
        let renders = attribute("charset").is_some()
            || attribute("name").is_some_and(|name| name == "viewport");
        (!renders && self.policy.metadata == Disposition::Strip)
            .then_some(HiddenContent::Properties)
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

/// One attribute of a tag: its key and raw value as written, and the byte range
/// the whole attribute (with its leading whitespace) occupies.
struct Attribute<'a> {
    key: &'a str,
    value: &'a str,
    range: Range<usize>,
}

/// Every attribute of `e` that could be placed in `raw`. Lenient: an attribute
/// quick-xml cannot parse, or one not borrowed from `raw`, is skipped.
fn attributes<'a>(raw: &'a str, e: &BytesStart<'a>) -> Vec<Attribute<'a>> {
    let base = raw.as_ptr() as usize;
    let offset = |slice: &str| {
        (slice.as_ptr() as usize)
            .checked_sub(base)
            .filter(|&at| at + slice.len() <= raw.len())
    };
    let mut out = Vec::new();
    for attr in e.html_attributes().with_checks(false).flatten() {
        let Some(key_start) = offset(attr.key.as_ref()) else {
            continue;
        };
        let Some(key) = raw.get(key_start..key_start + attr.key.as_ref().len()) else {
            continue;
        };
        let (value, end) = match &attr.value {
            // A value-less attribute (`<div hidden>`) reads as an empty value
            // that sits nowhere in the source; it ends with its key.
            Cow::Borrowed(value) if value.is_empty() && offset(value).is_none() => {
                ("", key_start + key.len())
            }
            Cow::Borrowed(value) => {
                let Some(start) = offset(value) else {
                    continue;
                };
                let end = start + value.len();
                let quoted = matches!(raw.as_bytes().get(end), Some(b'"' | b'\''));
                let Some(value) = raw.get(start..end) else {
                    continue;
                };
                (value, end + usize::from(quoted))
            }
            // A value-less attribute (`<input disabled>`) ends with its key.
            Cow::Owned(_) => ("", key_start + key.len()),
        };
        let start = raw[..key_start]
            .trim_end_matches(|c: char| c.is_ascii_whitespace())
            .len();
        out.push(Attribute {
            key,
            value,
            range: start..end,
        });
    }
    out
}

/// The ranges of the HTML element `name`'s attributes to remove, each with the
/// hidden content it holds: an `on*` event handler or a URL that runs script,
/// a URL the page loads from a server, a `ping` that reports a followed link,
/// and inline style that loads from a server.
fn hidden_attributes(raw: &str, e: &BytesStart<'_>, name: &str) -> Vec<Removal> {
    let hyperlink = name == "a" || name == "area";
    attributes(raw, e)
        .into_iter()
        .filter_map(|attr| {
            let key = attr.key.to_ascii_lowercase();
            let local = key.rsplit(':').next().unwrap_or(&key);
            let url = normalized_url(attr.value);
            let content = if key.starts_with("on")
                || SCRIPT_SCHEMES.iter().any(|scheme| url.starts_with(scheme))
            {
                HiddenContent::Script
            } else if local == "ping"
                || (LOADING_ATTRIBUTES.contains(&local) && loads_remote(attr.value, local))
                || (local == "href" && !hyperlink && is_remote(attr.value))
                || (local == "style" && css_loads_remote(&url))
            {
                HiddenContent::Link
            } else {
                return None;
            };
            Some(Removal {
                range: attr.range,
                content,
            })
        })
        .collect()
}

/// Whether the attribute `key`'s `value` loads anything from a server: a
/// `srcset` lists several URLs, every other attribute one.
fn loads_remote(value: &str, key: &str) -> bool {
    if key == "srcset" {
        value
            .split(',')
            .filter_map(|candidate| candidate.split_whitespace().next())
            .any(is_remote)
    } else {
        is_remote(value)
    }
}

/// Whether the URL `value` reaches a server.
fn is_remote(value: &str) -> bool {
    let url = normalized_url(value);
    REMOTE_SCHEMES.iter().any(|scheme| url.starts_with(scheme))
}

/// Whether the CSS `css` loads anything from a server, through a `url(...)` or
/// an `@import`.
fn css_loads_remote(css: &str) -> bool {
    let css = css.to_ascii_lowercase();
    let urls = css.split("url(").skip(1).map(|rest| {
        rest.split(')')
            .next()
            .unwrap_or_default()
            .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'')
    });
    let imports = css
        .split("@import")
        .skip(1)
        .map(|rest| rest.trim_start().trim_start_matches(['"', '\'']));
    urls.chain(imports).any(is_remote)
}

/// A URL attribute's `value` as a browser reads its scheme: character
/// references decoded, the tab and line breaks a URL parser drops removed, and
/// leading spaces and control characters trimmed, lowercased.
fn normalized_url(value: &str) -> String {
    let decoded = unescape(value).unwrap_or(Cow::Borrowed(value));
    decoded
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .skip_while(|c| *c <= ' ')
        .collect::<String>()
        .to_ascii_lowercase()
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
        let redact = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
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
        let redact = HiddenContentPolicy::default().with_metadata(Disposition::Redact);
        assert_eq!(sanitize(html(redact), raw), raw);
    }

    /// A redirect or a cookie acts when the page opens, so it goes whatever the
    /// metadata disposition, and a `javascript:` redirect with it.
    #[test]
    fn acting_meta_directives_go_under_every_policy() {
        let raw = r#"<head><meta http-equiv="refresh" content="0;url=javascript:steal()"><meta http-equiv="Set-Cookie" content="id=1"></head>"#;
        let redact = HiddenContentPolicy::default().with_metadata(Disposition::Redact);
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
        let redact = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
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
