//! [`MarkupSanitizer`]: remove a markup document's hidden content before its
//! text is extracted.
//!
//! The sanitizer finds the byte ranges of hidden content in the raw source and
//! cuts them out, leaving every other byte as it was, so the extraction that
//! follows (and its byte-faithful splice) runs over exactly what will ship:
//!
//! - **Active content**, always (HTML): `<script>`, `<object>`, `<applet>`, and
//!   `<embed>` elements, `on*` event-handler attributes, and attributes whose
//!   URL runs script (`javascript:`, `vbscript:`).
//! - **Collaboration**, under [`Disposition::Strip`]: comments, the
//!   author's notes a reader never sees.
//! - **Metadata**, under [`Disposition::Strip`] (HTML): `<meta>` fields,
//!   except the ones that render the page (its character set, an `http-equiv`
//!   directive, the viewport).
//!
//! A document type that declares entities is refused outright: an entity's
//! replacement text is never extracted, so it would ship past detection.

use std::borrow::Cow;
use std::ops::Range;

use elide_codec::StrippedContent;
use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};
use elide_core::{Error, ErrorKind, Result};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::markup_parser::{bom_len, end_name, local_name, malformed};

/// HTML elements that run code, removed with their whole subtree.
const ACTIVE_ELEMENTS: [&str; 3] = ["script", "object", "applet"];

/// HTML void elements that run code, removed as a single tag.
const ACTIVE_VOID_ELEMENTS: [&str; 1] = ["embed"];

/// URL schemes that run script when the URL is followed.
const SCRIPT_SCHEMES: [&str; 2] = ["javascript:", "vbscript:"];

/// The markup dialect a [`MarkupSanitizer`] reads: strict XML, or lenient HTML
/// with its element vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// Well-formed XML, which has no active content or metadata vocabulary.
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
    /// document type declares entities.
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
        let mut reader = Reader::from_str(raw);
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
        let mut removing: Option<ActiveRegion> = None;

        loop {
            let event = reader.read_event().map_err(malformed)?;
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
                    // An active element left open by lenient HTML runs to the
                    // end of the document.
                    Event::Eof => {
                        removals.push(region.removal(raw.len()));
                        break;
                    }
                    _ => {}
                }
                continue;
            }

            match &event {
                Event::Eof => break,
                Event::DocType(doctype) if doctype.as_ref().contains("<!ENTITY") => {
                    return Err(Error::new(
                        ErrorKind::MalformedInput,
                        "markup document type declares entities, whose text would bypass detection",
                    ));
                }
                Event::Comment(_) if self.policy.collaboration == Disposition::Strip => {
                    removals.push(Removal {
                        range: span,
                        content: HiddenContent::Comment,
                    });
                }
                Event::Start(e) | Event::Empty(e) if html => {
                    let name = local_name(e);
                    let is_start = matches!(event, Event::Start(_));
                    if is_start && ACTIVE_ELEMENTS.contains(&name.as_str()) {
                        removing = Some(ActiveRegion {
                            content: active_content(&name),
                            name,
                            depth: 1,
                            start: span.start,
                        });
                    } else if ACTIVE_ELEMENTS.contains(&name.as_str())
                        || ACTIVE_VOID_ELEMENTS.contains(&name.as_str())
                    {
                        removals.push(Removal {
                            range: span,
                            content: active_content(&name),
                        });
                    } else if name == "meta" && self.removes_meta(raw, e) {
                        removals.push(Removal {
                            range: span,
                            content: HiddenContent::Properties,
                        });
                    } else {
                        removals.extend(active_attributes(raw, e).into_iter().map(|range| {
                            Removal {
                                range,
                                content: HiddenContent::Script,
                            }
                        }));
                    }
                }
                _ => {}
            }
        }
        Ok(removals)
    }

    /// Whether the `<meta>` tag `e` is removed: metadata is stripped and the tag
    /// does not render the page.
    fn removes_meta(&self, raw: &str, e: &BytesStart<'_>) -> bool {
        if self.policy.metadata == Disposition::Redact {
            return false;
        }
        let renders = attributes(raw, e).iter().any(|attr| {
            let key = attr.key.to_ascii_lowercase();
            key == "charset"
                || key == "http-equiv"
                || (key == "name" && attr.value.eq_ignore_ascii_case("viewport"))
        });
        !renders
    }
}

/// A byte range of the source to remove, with the hidden content it holds.
struct Removal {
    range: Range<usize>,
    content: HiddenContent,
}

/// An active element being removed: its lowercased name, the hidden content it
/// is, the open depth of that same name (so a nested one resolves to the
/// matching close), and the byte where its start tag begins.
struct ActiveRegion {
    name: String,
    content: HiddenContent,
    depth: usize,
    start: usize,
}

impl ActiveRegion {
    /// The removal of this region, closed at byte `end`.
    fn removal(&self, end: usize) -> Removal {
        Removal {
            range: self.start..end,
            content: self.content,
        }
    }
}

/// The hidden content the active element `name` is: a `<script>` runs script,
/// every other one embeds a plugin control.
fn active_content(name: &str) -> HiddenContent {
    if name == "script" {
        HiddenContent::Script
    } else {
        HiddenContent::Control
    }
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
    let offset = |slice: &str| (slice.as_ptr() as usize).checked_sub(base);
    let mut out = Vec::new();
    for attr in e.attributes().with_checks(false).flatten() {
        let Some(key_start) = offset(attr.key.as_ref()) else {
            continue;
        };
        let Some(key) = raw.get(key_start..key_start + attr.key.as_ref().len()) else {
            continue;
        };
        let (value, end) = match &attr.value {
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

/// The ranges of `e`'s attributes that run script: an `on*` event handler, or
/// a URL with a script scheme.
fn active_attributes(raw: &str, e: &BytesStart<'_>) -> Vec<Range<usize>> {
    attributes(raw, e)
        .into_iter()
        .filter(|attr| {
            let key = attr.key.to_ascii_lowercase();
            let value = attr.value.trim_start().to_ascii_lowercase();
            key.starts_with("on")
                || SCRIPT_SCHEMES
                    .iter()
                    .any(|scheme| value.starts_with(scheme))
        })
        .map(|attr| attr.range)
        .collect()
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
        let raw = r#"<head><meta charset="utf-8"><meta name="viewport" content="w"><meta name="author" content="Jane Roe"><meta http-equiv="refresh" content="5"></head>"#;
        let out = sanitize(html(HiddenContentPolicy::default()), raw);
        assert_eq!(
            out,
            r#"<head><meta charset="utf-8"><meta name="viewport" content="w"><meta http-equiv="refresh" content="5"></head>"#
        );
        let redact = HiddenContentPolicy::default().with_metadata(Disposition::Redact);
        assert_eq!(sanitize(html(redact), raw), raw);
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
