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
//!   A `<textarea>` or `<title>` whose body holds markup, which a browser reads
//!   as text, is refused. An element left out ends where a browser ends it:
//!   at its end tag, at a start tag that closes it when HTML lets its end tag
//!   be left out, or where HTML breaks out of SVG and MathML.
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
//!   document type, or XInclude (whose namespace is compared with its
//!   references decoded); a document whose root is an XInclude is refused.
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
mod open;
mod url;

use std::collections::HashSet;
use std::ops::Range;

use elide_codec::StrippedContent;
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};
use elide_core::{Error, Result};
use quick_xml::NsReader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};

use self::html::{Element, RCDATA_ELEMENTS, Space, TEXT_ELEMENTS, VOID_ELEMENTS};
use self::open::OpenElements;
use self::url::css_loads_remote;
use crate::engine::parser::{bom_len, end_name, start_name};
use crate::engine::tag::{HtmlAttribute, HtmlTagExt};
use crate::references::{CharacterReferences, decode_references};

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
    /// The SVG and MathML a page has entered, and the HTML read inside them,
    /// innermost last.
    spaces: Vec<Foreign>,
    /// The HTML `<textarea>` or `<title>` whose body is being read.
    rcdata: Option<String>,
    /// Whether a kept HTML `<noscript>` is open.
    noscript: bool,
    /// How many XML elements are open.
    depth: usize,
}

/// Where an event falls against the element being left out.
enum Skip {
    /// Inside it, left out with it.
    Inside,
    /// After it: the event ends it and is read on its own.
    Outside,
    /// The document ended inside it.
    End,
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
            rcdata: None,
            noscript: false,
            depth: 0,
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
            let in_xinclude = match namespace {
                ResolveResult::Bound(Namespace(uri)) => is_xinclude(uri)?,
                _ => false,
            };
            let span = last..reader.buffer_position() as usize + bom;
            last = span.end;

            if self.dropping.is_some() {
                match self.skip(&event, &span)? {
                    Skip::Inside => continue,
                    Skip::End => break,
                    Skip::Outside => {}
                }
            }
            // With script on, a browser reads a `<noscript>` body as text up
            // to its first `</noscript`, which markup copied as written must
            // not hold, nor a style sheet read as text.
            if self.noscript {
                let in_style = self.style.as_ref().is_some_and(|sheet| sheet.raw);
                match &event {
                    Event::End(e) if !in_style && end_name(e, raw, true)? == "noscript" => {
                        self.noscript = false;
                    }
                    _ if ends_text(&raw[span.clone()], "noscript") => {
                        return Err(Error::malformed(
                            "a browser ends this HTML <noscript> at a `</noscript` inside markup, so what follows it renders",
                        ));
                    }
                    _ => {}
                }
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
            // A browser reads a `<textarea>` or `<title>` body as text, so
            // markup this reader sees inside it would be rebuilt as what it
            // is not.
            if let Some(element) = self.rcdata.as_deref() {
                match &event {
                    Event::Text(_) | Event::GeneralRef(_) | Event::Eof => {}
                    Event::End(e) if end_name(e, raw, true)? == element => self.rcdata = None,
                    _ => {
                        return Err(Error::malformed(format!(
                            "a browser reads the markup inside this HTML <{element}> as its text"
                        )));
                    }
                }
            }
            match &event {
                Event::Eof => break,
                Event::Text(text) => {
                    if let Some(sheet) = self.style.as_mut() {
                        sheet.css.push_str(text.as_ref());
                    }
                    if html {
                        self.copy(span);
                    } else {
                        self.copy_xml_text(span);
                    }
                }
                Event::GeneralRef(_) => {
                    // A style sheet in SVG or MathML is markup, whose
                    // references a browser decodes before reading the CSS.
                    if let Some(sheet) = self.style.as_mut() {
                        let (decoded, _) =
                            decode_references(&raw[span.clone()], 0, CharacterReferences::Html);
                        sheet.css.push_str(&decoded);
                    }
                    self.copy(span);
                }
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
                        if self.depth == 0 {
                            return Err(Error::malformed(
                                "the XML document element is an XInclude, which leaves no document",
                            ));
                        }
                        self.record(HiddenContent::Link);
                        if is_start {
                            self.dropping =
                                Some(Region::new(start_name(e, raw, false)?, Space::Html));
                        }
                    } else {
                        self.depth += usize::from(is_start);
                        self.copy(span);
                    }
                }
                Event::End(e) => {
                    if html {
                        self.html_end(e.html5_tag(raw)?.name);
                    } else {
                        self.depth = self.depth.saturating_sub(1);
                        self.copy(span);
                    }
                }
            }
        }
        // A style sheet left open by lenient HTML runs to the end.
        self.close_style();
        Ok(())
    }

    /// Follow `event` inside the element being left out, which ends at its
    /// own end tag past the elements opened inside it, or where a browser
    /// ends it without one:
    ///
    /// - in HTML, at a start tag that closes it when nothing inside it is
    ///   open (`<p hidden>a<p>b` closes the first paragraph);
    /// - in SVG or MathML, at an element that breaks out to HTML, unless an
    ///   element inside it reads its content as HTML.
    fn skip(&mut self, event: &Event<'_>, span: &Range<usize>) -> Result<Skip> {
        let html = self.html();
        let raw = self.raw;
        let Some(region) = self.dropping.as_mut() else {
            return Ok(Skip::Outside);
        };
        // Inside a text element, only its own end tag is markup.
        if let Some(text) = region.text.as_deref() {
            let ends = match event {
                Event::End(e) => text != "plaintext" && end_name(e, raw, true)? == text,
                _ => false,
            };
            if !ends && !matches!(event, Event::Eof) {
                return Ok(Skip::Inside);
            }
            region.text = None;
        }
        match event {
            // An element left open by lenient HTML runs to the end of the
            // document.
            Event::Eof => return Ok(Skip::End),
            Event::Start(e) | Event::Empty(e) if html => {
                let tag = e.html5_tag(raw)?;
                let name = tag.name.to_ascii_lowercase();
                // An HTML element in SVG or MathML closes the foreign elements
                // open around it, up to one that reads its content as HTML;
                // past the removed element itself, it ends the removal.
                if html::breaks_out(region.current().1, &name, &tag.attributes) {
                    while let Some((open, space)) = region.open.last()
                        && *space != Space::Html
                        && !html::integration_point(*space, open)
                    {
                        region.open.pop();
                    }
                    if region.open.is_empty()
                        && region.space != Space::Html
                        && !html::integration_point(region.space, &region.name)
                    {
                        self.dropping = None;
                        return Ok(Skip::Outside);
                    }
                }
                // The vocabulary the element is in: HTML where the current
                // element reads its content as HTML, but for an `svg` or
                // `math` root; else the current element's, but for an `svg`
                // in a MathML `annotation-xml`.
                let (current, current_space) = region.current();
                let as_html = current_space == Space::Html
                    || (html::integration_point(current_space, current)
                        && !(current_space == Space::Math && html::stays_math(&name)));
                let space = match name.as_str() {
                    "svg" if as_html || current == "annotation-xml" => Space::Svg,
                    "math" if as_html => Space::Math,
                    _ if as_html => Space::Html,
                    _ => current_space,
                };
                if space == Space::Html {
                    // An HTML start tag closes the elements open in the
                    // removed one whose end it implies, void or self-closed
                    // as it may be; if that leaves nothing open and it
                    // implies the removed element's end too, it ends the
                    // removal (`<p hidden>a<p>b`).
                    close_implied(&mut region.open, &name);
                    if region.space == Space::Html
                        && region.open.is_empty()
                        && html::closes(&region.name, &name)
                    {
                        self.dropping = None;
                        return Ok(Skip::Outside);
                    }
                }
                // A void HTML element never holds anything; a browser ignores
                // the self-closing flag on any other HTML element, and reads
                // a `/` an unquoted value took as its own.
                let opens = if space == Space::Html {
                    !VOID_ELEMENTS.contains(&name.as_str())
                } else {
                    !self_closing(raw, span, &tag.attributes, matches!(event, Event::Start(_)))
                };
                if opens {
                    if space == Space::Html && TEXT_ELEMENTS.contains(&name.as_str()) {
                        region.text = Some(name.clone());
                    }
                    region.open.push(name, space);
                }
            }
            Event::Start(e) => {
                let space = region.space;
                region.open.push(start_name(e, raw, false)?, space);
            }
            Event::End(e) => {
                let name = end_name(e, raw, html)?;
                if !region.open.close(&name) && name == region.name {
                    self.dropping = None;
                }
            }
            _ => {}
        }
        Ok(Skip::Inside)
    }

    /// Copy the XML text at `span`. A `]]>` is not allowed in text, so where
    /// a removal joins `]` to `]>` its `>` is written as a reference.
    fn copy_xml_text(&mut self, span: Range<usize>) {
        let text = &self.raw[span];
        let before = self
            .out
            .bytes()
            .rev()
            .take(2)
            .take_while(|&b| b == b']')
            .count();
        let lead = text.bytes().take_while(|&b| b == b']').count();
        if before > 0 && before + lead >= 2 && text.as_bytes().get(lead) == Some(&b'>') {
            self.out.push_str(&text[..lead]);
            self.out.push_str("&gt;");
            self.out.push_str(&text[lead + 1..]);
        } else {
            self.out.push_str(text);
        }
    }

    /// Leave the SVG and MathML the reader is in, back to the HTML around
    /// them, closing a style sheet read inside them. Returns whether there
    /// was any to leave.
    fn leave_foreign(&mut self) -> bool {
        let mut left = false;
        while self
            .spaces
            .last()
            .is_some_and(|foreign| foreign.space != Space::Html)
        {
            self.spaces.pop();
            left = true;
        }
        if left {
            self.close_style();
        }
        left
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
            // The body is raw text, read as written: a comment, a tag or a
            // reference in it is text too, but for a `</style` in it, which a
            // browser ends the body at.
            _ => {
                if ends_text(&self.raw[span.clone()], "style") {
                    return Err(Error::malformed(
                        "a browser ends this HTML <style> at a `</style` inside markup, so what follows it renders",
                    ));
                }
                if let Some(sheet) = self.style.as_mut() {
                    sheet.css.push_str(&self.raw[span.clone()]);
                }
            }
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
        let left = html::breaks_out(self.space(), &name, &tag.attributes) && self.leave_foreign();
        // A MathML token element reads HTML, but for an `mglyph` or
        // `malignmark` started right in it, which stays MathML.
        let space = match self.spaces.last() {
            Some(foreign)
                if foreign.space == Space::Html
                    && foreign.open.is_empty()
                    && html::integration_point(Space::Math, &foreign.root)
                    && html::stays_math(&name) =>
            {
                Space::Math
            }
            _ => self.space(),
        };
        // A start tag that implies the end of the HTML element open right in
        // the element read as HTML closes it, kept or not.
        if space == Space::Html
            && let Some(foreign) = self.spaces.last_mut()
            && foreign.space == Space::Html
        {
            close_implied(&mut foreign.open, &name);
        }
        let element = html::element(space, &name, &tag.attributes, self.sanitizer.policy);
        let self_closing = self_closing(self.raw, &span, &tag.attributes, is_start);
        // Whether the element holds what follows: a browser ignores the
        // self-closing flag on a non-void HTML element, but not on the root of
        // SVG or MathML, kept or not.
        let foreign_root = matches!(name.as_str(), "svg" | "math");
        let opens = match element {
            Element::Keep | Element::Drop(_) if space == Space::Html && !foreign_root => {
                !VOID_ELEMENTS.contains(&name.as_str())
            }
            _ => !self_closing,
        };
        if let Element::Drop(content) = element {
            self.record(content);
            if left {
                // The output leaves SVG or MathML where the source does, so a
                // rebuild of it reads what follows the same way.
                self.out.push_str("<span></span>");
            }
            if opens {
                // A root's content is read in its own vocabulary, so an HTML
                // element in it ends it as it would end the kept root.
                let inside = match name.as_str() {
                    "svg" if space == Space::Html => Space::Svg,
                    "math" if space == Space::Html => Space::Math,
                    _ => space,
                };
                let mut region = Region::new(name, inside);
                if inside == Space::Html && TEXT_ELEMENTS.contains(&region.name.as_str()) {
                    region.text = Some(region.name.clone());
                }
                self.dropping = Some(region);
            }
            return Ok(());
        };

        // The root of SVG or MathML carries that vocabulary's attributes.
        let attributes_space = match element {
            Element::Enter(entered) => entered,
            _ => space,
        };
        let style_start = self.out.len();
        self.out.push('<');
        self.out.push_str(tag.name);
        let mut seen = HashSet::with_capacity(tag.attributes.len());
        let mut last: Option<&HtmlAttribute<'_>> = None;
        for attr in &tag.attributes {
            let key = attr.name.to_ascii_lowercase();
            // A browser reads the first of a repeated attribute and ignores
            // the rest, which still carry their value, even when the first is
            // left out.
            let content = if seen.insert(key) {
                html::attribute(attributes_space, &name, attr)
            } else {
                Some(HiddenContent::Undisplayed)
            };
            if let Some(content) = content {
                self.record(content);
                continue;
            }
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

        if !opens {
            return Ok(());
        }
        match element {
            Element::Enter(space) => self.spaces.push(Foreign::new(space, name.clone())),
            _ => {
                if let Some(foreign) = self.spaces.last_mut() {
                    if foreign.root == name {
                        foreign.depth += 1;
                    } else if foreign.space == Space::Html {
                        foreign.open.push(name.clone(), ());
                    }
                }
                // Inside an SVG `desc` or a MathML `mi`, a browser reads HTML
                // until the element ends.
                if html::integration_point(space, &name) {
                    self.spaces.push(Foreign::new(Space::Html, name.clone()));
                }
            }
        }
        if space == Space::Html && name == "noscript" {
            self.noscript = true;
        }
        if space == Space::Html && RCDATA_ELEMENTS.contains(&name.as_str()) {
            self.rcdata = Some(name.clone());
        }
        if name == "style" {
            // A style sheet in another leaves the outer one's end unchecked.
            if self.style.is_some() {
                return Err(Error::malformed(
                    "a style sheet nested in another is not read as one",
                ));
            }
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
            // An HTML element open inside the element read as HTML ends first.
            if foreign.open.close(&name) {
                return false;
            }
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

/// Whether the HTML tag read at `span`, with `attributes`, carries the
/// self-closing flag: a `/` before the `>`, unless an unquoted value read it
/// as its own.
fn self_closing(
    raw: &str,
    span: &Range<usize>,
    attributes: &[HtmlAttribute<'_>],
    is_start: bool,
) -> bool {
    let slash = span.end.saturating_sub(2);
    !is_start
        && !attributes.last().is_some_and(|attr| {
            raw.substr_range(attr.source)
                .is_some_and(|range| range.end > slash)
        })
}

/// Close the HTML elements innermost in `open` whose end the start tag
/// `name` implies, as a second `<p>` ends the first.
fn close_implied<T>(open: &mut OpenElements<T>, name: &str) {
    while open
        .last()
        .is_some_and(|(open, _)| html::closes(open, name))
    {
        open.pop();
    }
}

/// An element being left out with its subtree: its lowercased name, the
/// vocabulary it was read in, and the elements open inside it, so its own end
/// tag is told from theirs.
struct Region {
    name: String,
    space: Space,
    open: OpenElements<Space>,
    /// The text element, open in it or itself, whose body a browser reads
    /// as text up to its end tag.
    text: Option<String>,
}

impl Region {
    /// The element whose content is being read, with its vocabulary: the
    /// innermost one open in it, else itself.
    fn current(&self) -> (&str, Space) {
        self.open
            .last()
            .map_or((self.name.as_str(), self.space), |(name, space)| {
                (name.as_str(), *space)
            })
    }

    fn new(name: String, space: Space) -> Self {
        Self {
            name,
            space,
            open: OpenElements::new(),
            text: None,
        }
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

/// SVG or MathML entered by its root element, or HTML read inside an element
/// of theirs, and the open depth of that element's name.
struct Foreign {
    space: Space,
    root: String,
    depth: usize,
    /// Under HTML read inside an SVG or MathML element: the HTML elements
    /// open in it, so what is started right in that element is told apart.
    open: OpenElements,
}

impl Foreign {
    fn new(space: Space, root: String) -> Self {
        Self {
            space,
            root,
            depth: 1,
            open: OpenElements::new(),
        }
    }
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

/// Whether `text` holds the start of an end tag for the element `name`
/// (lowercased), in any case.
fn ends_text(text: &str, name: &str) -> bool {
    let len = name.len() + 2;
    text.as_bytes().windows(len).any(|window| {
        window.starts_with(b"</") && window[2..].eq_ignore_ascii_case(name.as_bytes())
    })
}

/// Whether the namespace `uri`, as written, is XInclude's once its character
/// references are decoded.
fn is_xinclude(uri: &str) -> Result<bool> {
    if !uri.contains('&') {
        return Ok(uri == XINCLUDE);
    }
    let decoded = quick_xml::escape::unescape(uri)
        .map_err(|e| Error::malformed(format!("malformed markup: namespace {e}")))?;
    Ok(decoded == XINCLUDE)
}

/// Whether a comment's `text` is a conditional comment (`[if IE]>…`), markup
/// an old browser parses and runs.
fn is_conditional(text: &str) -> bool {
    text.trim_start().starts_with("[if")
}

#[cfg(test)]
mod tests;
