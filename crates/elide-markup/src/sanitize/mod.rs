//! [`MarkupSanitizer`]: rebuild a markup document from what it renders before
//! its text is extracted.
//!
//! The rebuild reads the source once and writes a new document from an
//! allowlist, so the extraction that follows (and its byte-faithful splice)
//! runs over exactly what will ship. Text, attribute values, and kept comments
//! are copied as written, so their character references read as they did;
//! everything the allowlist does not name is left out and recorded by what it
//! is.
//!
//! - **HTML**: each element is copied only if it renders in the vocabulary it
//!   is read in (HTML, or the SVG and MathML a page embeds, entered and left
//!   as a browser does), keeping only the attributes that element renders
//!   with, each written once (see [`html`]). An element the allowlist does not
//!   name goes with everything inside it, as do:
//!   - **active content**: `<script>`, plugin objects, frames, SVG animation;
//!   - **content never displayed**: a `<template>`, a closed `<dialog>`, a
//!     hidden `<input>`, an element marked `hidden` (one only hidden until
//!     found stays), MathML phantoms and annotations, and what a browser reads
//!     as a comment: a CDATA section or processing instruction in HTML;
//!   - **external references**: a `<base>`, a remote `<link>` or style sheet.
//!
//!   Of a kept element's attributes, `on*` event handlers and URLs that run
//!   script (`javascript:`, `vbscript:`, `data:text/html`, read the way a
//!   browser reads them), what the page loads from a server when opened, a
//!   hyperlink's `ping`, and inline style that loads from a server go even
//!   when the allowlist names them. A `<meta>` that acts when the page opens
//!   (a redirect, a cookie) goes always, and one that describes the page under
//!   [`KeepOnlyEssential`](elide_core::redaction::Disposition::KeepOnlyEssential)
//!   metadata.
//! - **XML**: the document as parsed, without processing instructions, its
//!   document type, or XInclude.
//! - **Comments** go under
//!   [`KeepOnlyEssential`](elide_core::redaction::Disposition::KeepOnlyEssential)
//!   collaboration; a conditional comment, markup an old browser runs, always.
//!
//! A document type that declares entities, or in XML names an external one, is
//! refused outright: the text its references stand for is never extracted, so
//! it would ship past detection.
//!
//! The rebuild is a fixed point: rebuilding what it wrote writes the same
//! bytes, which is how an encoded document is [`check`](MarkupSanitizer::check)ed.

mod html;
mod url;

use std::ops::Range;

use elide_codec::StrippedContent;
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};
use elide_core::{Error, Result};
use quick_xml::NsReader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};

use self::html::{Element, Space, VOID_ELEMENTS};
use self::url::css_loads_remote;
use crate::engine::parser::{bom_len, end_name, start_name};
use crate::engine::tag::{HtmlAttribute, HtmlTagExt};

/// The XInclude namespace, whose `include` element pulls in another document.
const XINCLUDE: &str = "http://www.w3.org/2001/XInclude";

/// The markup dialect a [`MarkupSanitizer`] reads: strict XML, or lenient HTML
/// with its element vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// Well-formed XML, kept as parsed.
    Xml,
    /// HTML, read leniently and rebuilt from what renders.
    Html,
}

/// Rebuilds a markup document under a [`HiddenContentPolicy`].
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

    /// `raw` rebuilt from its allowlist, and the record of what was left out.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
    /// markup cannot be read, a browser would read it differently from this
    /// reader, or its document type declares entities or (in XML) names an
    /// external one.
    pub(crate) fn sanitize(&self, raw: String) -> Result<(String, StrippedContent)> {
        let mut rebuild = Rebuild::new(self, &raw);
        rebuild.run()?;
        Ok((rebuild.out, rebuild.stripped))
    }

    /// Check that `raw` is what a rebuild writes: rebuilding it changes
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) if a
    /// rebuild would change `raw`, or the errors of [`sanitize`](Self::sanitize).
    pub(crate) fn check(&self, raw: &str) -> Result<()> {
        let mut rebuild = Rebuild::new(self, raw);
        rebuild.run()?;
        if rebuild.out == raw {
            return Ok(());
        }
        let at = raw
            .bytes()
            .zip(rebuild.out.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| raw.len().min(rebuild.out.len()));
        let what = rebuild
            .stripped
            .iter()
            .next()
            .map_or_else(String::new, |entry| format!(" ({:?})", entry.content));
        Err(Error::sanitization(format!(
            "encoded markup is not what a rebuild writes: it differs at byte {at}{what}"
        )))
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

/// One rebuild of a document: the source, what has been written, and where
/// the reader is.
struct Rebuild<'a> {
    sanitizer: &'a MarkupSanitizer,
    raw: &'a str,
    out: String,
    stripped: StrippedContent,
    /// The element being left out with its subtree.
    dropping: Option<Region>,
    /// The `<style>` element being read.
    style: Option<StyleSheet>,
    /// The SVG and MathML a page has entered, innermost last.
    spaces: Vec<Foreign>,
}

impl<'a> Rebuild<'a> {
    fn new(sanitizer: &'a MarkupSanitizer, raw: &'a str) -> Self {
        Self {
            sanitizer,
            raw,
            out: String::with_capacity(raw.len()),
            stripped: StrippedContent::new(),
            dropping: None,
            style: None,
            spaces: Vec::new(),
        }
    }

    fn html(&self) -> bool {
        self.sanitizer.dialect == Dialect::Html
    }

    /// The vocabulary the reader is in.
    fn space(&self) -> Space {
        self.spaces
            .last()
            .map_or(Space::Html, |foreign| foreign.space)
    }

    fn record(&mut self, content: HiddenContent) {
        self.stripped.record(content, None, 1);
    }

    fn copy(&mut self, span: Range<usize>) {
        self.out.push_str(&self.raw[span]);
    }

    fn run(&mut self) -> Result<()> {
        let raw = self.raw;
        let html = self.html();
        let mut reader = NsReader::from_str(raw);
        if html {
            let config = reader.config_mut();
            config.check_end_names = false;
            config.allow_unmatched_ends = true;
            config.allow_dangling_amp = true;
        }
        // quick-xml reports positions past a leading BOM, which is copied
        // ahead of the document.
        let bom = bom_len(raw);
        self.out.push_str(&raw[..bom]);
        let mut last = bom;

        loop {
            let (namespace, event) = reader
                .read_resolved_event()
                .map_err(|e| Error::malformed(format!("malformed markup: {e}")))?;
            let in_xinclude =
                matches!(namespace, ResolveResult::Bound(Namespace(uri)) if uri == XINCLUDE);
            let span = last..reader.buffer_position() as usize + bom;
            last = span.end;

            if self.dropping.is_some() {
                if self.skip(&event)? {
                    break;
                }
                continue;
            }
            if self.style.as_ref().is_some_and(|sheet| sheet.raw) {
                if self.style_text(&event, span)? {
                    break;
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
                Event::Text(text) => {
                    if let Some(sheet) = self.style.as_mut() {
                        sheet.css.push_str(text.as_ref());
                    }
                    self.copy(span);
                }
                Event::GeneralRef(_) => self.copy(span),
                Event::CData(text) => {
                    // A browser reads a CDATA section as one only in SVG and
                    // MathML; in HTML it is a comment.
                    if html && self.space() == Space::Html {
                        self.record(HiddenContent::Undisplayed);
                    } else {
                        if let Some(sheet) = self.style.as_mut() {
                            sheet.css.push_str(text.as_ref());
                        }
                        self.copy(span);
                    }
                }
                Event::Decl(_) | Event::PI(_) if html => {
                    // A browser reads a processing instruction as a comment.
                    self.record(HiddenContent::Undisplayed);
                }
                Event::Decl(_) => self.copy(span),
                Event::PI(instruction) => {
                    let content = if instruction.as_ref().starts_with("xml-stylesheet") {
                        HiddenContent::Link
                    } else {
                        HiddenContent::ApplicationData
                    };
                    self.record(content);
                }
                Event::DocType(doctype) => {
                    self.sanitizer.check_doctype(doctype.as_ref())?;
                    if html {
                        self.copy(span);
                    } else {
                        self.record(HiddenContent::ApplicationData);
                    }
                }
                // A conditional comment is markup an old browser runs, whatever
                // the policy does with comments.
                Event::Comment(text) if html && is_conditional(text.as_ref()) => {
                    self.record(HiddenContent::Script);
                }
                Event::Comment(_) => {
                    if self.sanitizer.policy.collaboration.treats_as_content() {
                        self.copy(span);
                    } else {
                        self.record(HiddenContent::Comment);
                    }
                }
                Event::Start(e) | Event::Empty(e) => {
                    let is_start = matches!(event, Event::Start(_));
                    if html {
                        self.html_start(e, span, is_start)?;
                    } else if in_xinclude {
                        self.record(HiddenContent::Link);
                        if is_start {
                            self.dropping = Some(Region::new(start_name(e, raw, false)?));
                        }
                    } else {
                        self.copy(span);
                    }
                }
                Event::End(e) => {
                    if html {
                        self.html_end(e.html5_tag(raw)?.name);
                    } else {
                        self.copy(span);
                    }
                }
            }
        }
        // A style sheet left open by lenient HTML runs to the end.
        self.close_style();
        Ok(())
    }

    /// Follow `event` inside the element being left out: only that element's
    /// own depth counts. Returns whether the document ended.
    fn skip(&mut self, event: &Event<'_>) -> Result<bool> {
        let html = self.html();
        let Some(region) = self.dropping.as_mut() else {
            return Ok(false);
        };
        match event {
            Event::Start(e) if start_name(e, self.raw, html)? == region.name => {
                region.depth += 1;
            }
            Event::End(e) if end_name(e, self.raw, html)? == region.name => {
                region.depth -= 1;
                if region.depth == 0 {
                    self.dropping = None;
                }
            }
            // An element left open by lenient HTML runs to the end of the
            // document.
            Event::Eof => return Ok(true),
            _ => {}
        }
        Ok(false)
    }

    /// Copy `event` inside an HTML `<style>`, whose body a browser reads as
    /// text, up to its end tag. Returns whether the document ended.
    fn style_text(&mut self, event: &Event<'_>, span: Range<usize>) -> Result<bool> {
        match event {
            Event::Eof => return Ok(true),
            Event::End(e) if end_name(e, self.raw, true)? == "style" => {
                self.copy(span);
                self.close_style();
                return Ok(false);
            }
            Event::Text(text) => {
                if let Some(sheet) = self.style.as_mut() {
                    sheet.css.push_str(text.as_ref());
                }
            }
            Event::CData(text) => {
                if let Some(sheet) = self.style.as_mut() {
                    sheet.css.push_str(text.as_ref());
                }
            }
            _ => {}
        }
        self.copy(span);
        Ok(false)
    }

    /// Close the `<style>` being read: one that loads from a server goes whole.
    fn close_style(&mut self) {
        if let Some(sheet) = self.style.take()
            && css_loads_remote(&sheet.css)
        {
            self.out.truncate(sheet.start);
            self.record(HiddenContent::Link);
        }
    }

    /// Rebuild the HTML start tag `e`, read at `span`.
    fn html_start(&mut self, e: &BytesStart<'_>, span: Range<usize>, is_start: bool) -> Result<()> {
        let tag = e.html5_tag(self.raw)?;
        let name = tag.name.to_ascii_lowercase();
        if html::breaks_out(self.space(), &name, &tag.attributes) {
            self.spaces.clear();
        }
        let space = self.space();
        let element = html::element(space, &name, &tag.attributes, self.sanitizer.policy);
        if let Element::Drop(content) = element {
            self.record(content);
            let void = space == Space::Html && VOID_ELEMENTS.contains(&name.as_str());
            if is_start && !void {
                self.dropping = Some(Region::new(name));
            }
            return Ok(());
        };

        // A `/` before the `>` is the self-closing flag unless an unquoted
        // value read it as its own.
        let slash = span.end.saturating_sub(2);
        let self_closing = !is_start
            && !tag.attributes.last().is_some_and(|attr| {
                self.raw
                    .substr_range(attr.source)
                    .is_some_and(|range| range.end > slash)
            });
        // The root of SVG or MathML carries that vocabulary's attributes.
        let attributes_space = match element {
            Element::Enter(entered) => entered,
            _ => space,
        };
        let style_start = self.out.len();
        self.out.push('<');
        self.out.push_str(tag.name);
        let mut seen: Vec<String> = Vec::new();
        let mut last: Option<&HtmlAttribute<'_>> = None;
        for attr in &tag.attributes {
            let key = attr.name.to_ascii_lowercase();
            // A browser reads the first of a repeated attribute and ignores
            // the rest, which still carry their value.
            let content = html::attribute(attributes_space, &name, attr)
                .or_else(|| seen.contains(&key).then_some(HiddenContent::Undisplayed));
            if let Some(content) = content {
                self.record(content);
                continue;
            }
            seen.push(key);
            self.out.push(' ');
            self.out.push_str(attr.source);
            last = Some(attr);
        }
        if self_closing {
            // An unquoted value would read a `/` right after it as its own.
            let unquoted = last
                .is_some_and(|attr| attr.value.is_some() && !attr.source.ends_with(['"', '\'']));
            self.out.push_str(if unquoted { " />" } else { "/>" });
        } else {
            self.out.push('>');
        }

        if !is_start {
            return Ok(());
        }
        match element {
            Element::Enter(space) => self.spaces.push(Foreign {
                space,
                root: name.clone(),
                depth: 1,
            }),
            _ => {
                if let Some(foreign) = self.spaces.last_mut()
                    && foreign.root == name
                {
                    foreign.depth += 1;
                }
            }
        }
        if name == "style" {
            self.style = Some(StyleSheet {
                start: style_start,
                css: String::new(),
                raw: space == Space::Html,
            });
        }
        Ok(())
    }

    /// Rebuild the HTML end tag named `written`.
    fn html_end(&mut self, written: &str) {
        let name = written.to_ascii_lowercase();
        let space = self.space();
        // An end tag holds nothing, so one the allowlist does not name is
        // left out unrecorded; a stray HTML one in SVG or MathML still acts.
        let named = html::names(space, &name) || html::names(Space::Html, &name);
        let leaves = self.spaces.last_mut().is_some_and(|foreign| {
            if foreign.root == name {
                foreign.depth -= 1;
            }
            foreign.depth == 0
        });
        if leaves {
            self.spaces.pop();
        }
        if name == "style" && space != Space::Html {
            self.out.push_str("</");
            self.out.push_str(written);
            self.out.push('>');
            self.close_style();
            return;
        }
        if named || leaves {
            self.out.push_str("</");
            self.out.push_str(written);
            self.out.push('>');
        }
    }
}

/// An element being left out with its subtree: its lowercased name, and the
/// open depth of that same name, so a nested one resolves to the matching
/// close.
struct Region {
    name: String,
    depth: usize,
}

impl Region {
    fn new(name: String) -> Self {
        Self { name, depth: 1 }
    }
}

/// A `<style>` element being read: where its start tag begins in the output,
/// its text so far, and whether a browser reads its body as text (in HTML) or
/// as markup (in SVG).
struct StyleSheet {
    start: usize,
    css: String,
    raw: bool,
}

/// SVG or MathML entered by its root element, and the open depth of that
/// root's name.
struct Foreign {
    space: Space,
    root: String,
    depth: usize,
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
mod tests;
