//! The HTML half of the sanitizer: which elements go whole, which attributes go,
//! and what a `<meta>` tag is.

use std::ops::Range;

use elide_core::redaction::{Disposition, HiddenContent};
use elide_core::{Error, Result};

use super::url::{SCRIPT_SCHEMES, css_loads_remote, is_remote, loads_remote, normalized_url};
use super::{MarkupSanitizer, Removal, Verdict};
use crate::engine::tag::HtmlTag;

/// HTML elements that run or embed active content, removed with their whole
/// subtree: a script, a plugin object, and a frame that loads another page or
/// carries one inline (`srcdoc`).
const ACTIVE_ELEMENTS: [&str; 6] = ["script", "object", "applet", "iframe", "frame", "frameset"];

/// HTML void elements: they have no end tag, so removing one removes its tag.
const VOID_ELEMENTS: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

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

impl MarkupSanitizer {
    /// What happens to the HTML element `e` named `name`.
    pub(super) fn html_verdict(
        &self,
        attributes: &[Attribute<'_>],
        name: &str,
        is_start: bool,
    ) -> Verdict {
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
                if let Some(content) = self.meta_removal(attributes) {
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

/// One attribute of a tag: its key and raw value as written, and the byte range
/// that removes it: the attribute with what separates it from the one before.
pub(super) struct Attribute<'a> {
    key: &'a str,
    value: &'a str,
    range: Range<usize>,
}

/// Every attribute of the HTML `tag`, read from `raw`, placed in it.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) when a
/// browser reads the tag differently from quick-xml, or an attribute cannot be
/// placed in `raw`: an attribute the sanitizer cannot see is one it cannot
/// remove.
pub(super) fn attributes<'a>(raw: &'a str, tag: &HtmlTag<'a>) -> Result<Vec<Attribute<'a>>> {
    let place = |slice: &str| {
        (slice.as_ptr() as usize)
            .checked_sub(raw.as_ptr() as usize)
            .filter(|&at| at + slice.len() <= raw.len())
            .map(|at| at..at + slice.len())
            .ok_or_else(|| Error::malformed("an HTML tag is not in the source"))
    };
    let bytes = raw.as_bytes();
    let mut previous = place(tag.name)?.end;
    let mut out = Vec::new();
    for attr in &tag.attributes {
        let source = place(attr.source)?;
        // The removal takes the whitespace and stray `/`s before the attribute,
        // back to the one before it, and keeps one when the attribute runs
        // straight into the next, so the two around it never join.
        let mut from = raw[..source.start]
            .trim_end_matches(|c: char| c.is_ascii_whitespace() || c == '/')
            .len()
            .max(previous);
        let joins_next = bytes
            .get(source.end)
            .is_some_and(|&b| !b.is_ascii_whitespace() && b != b'/' && b != b'>');
        if joins_next && from < source.start {
            from += 1;
        }
        out.push(Attribute {
            key: attr.name,
            value: attr.value.unwrap_or_default(),
            range: from..source.end,
        });
        previous = source.end;
    }
    Ok(out)
}

/// The ranges of the HTML element `name`'s attributes to remove, each with the
/// hidden content it holds: an `on*` event handler or a URL that runs script,
/// a URL the page loads from a server, a `ping` that reports a followed link,
/// and inline style that loads from a server.
pub(super) fn hidden_attributes(attributes: Vec<Attribute<'_>>, name: &str) -> Vec<Removal> {
    let hyperlink = name == "a" || name == "area";
    attributes
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
