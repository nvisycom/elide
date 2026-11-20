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

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use elide_codec::StrippedContent;
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};
use elide_core::{Error, Result};
use quick_xml::NsReader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};

use self::html::{Element, Space};
use self::open::OpenElements;
use self::url::css_loads_remote;
use crate::engine::config::Dialect;
use crate::engine::reader::{EventSpans, configure, malformed_markup};
use crate::engine::tag::{HtmlAttribute, HtmlTag, HtmlTagExt, tag_name};
use crate::references::{CharacterReferences, decoded};

/// The XInclude namespace, whose `include` element pulls in another document.
const XINCLUDE: &str = "http://www.w3.org/2001/XInclude";

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
    /// Where the output ends in a `&` and the letters, digits, or `#` after
    /// it, copied from text: the `&`'s offset and the output's length then.
    dangling: Option<(usize, usize)>,
    /// The HTML `<textarea>` or `<title>` whose body is being read.
    rcdata: Option<&'static str>,
    /// Whether a kept HTML `<noscript>` is open.
    noscript: bool,
    /// How many XML elements are open.
    depth: usize,
}

/// Where an event falls against the element being left out.
enum Skip {
    /// Inside it, left out with it.
    Inside,
    /// Its end: the event ends it and is left out with it.
    Last,
    /// After it: the event ends it and is read on its own.
    Outside,
    /// The document ended inside it.
    End,
}

/// What the rebuild does with an event once a text body has read it.
enum Flow {
    /// Rebuild it.
    Rebuild,
    /// It is done with: read the next one.
    Next,
    /// The document ended.
    Stop,
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
            dangling: None,
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

    /// Whether the element open is surely SVG or MathML, though it read its
    /// content as HTML: an `mtext` with nothing open in it. A browser reads a
    /// CDATA section there as one, elsewhere in HTML as a comment.
    fn in_foreign_element(&self) -> bool {
        self.spaces.last().is_some_and(|foreign| {
            foreign.space != Space::Html || (foreign.open.is_empty() && foreign.exact)
        })
    }

    /// Whether an HTML `<style>`, whose body a browser reads as text, is open.
    fn in_raw_style(&self) -> bool {
        self.style.as_ref().is_some_and(|sheet| sheet.as_text)
    }

    fn record(&mut self, content: HiddenContent) {
        self.stripped.record(content, None, 1);
    }

    fn copy(&mut self, span: Range<usize>) {
        self.out.push_str(&self.raw[span]);
    }

    fn run(&mut self) -> Result<()> {
        let raw = self.raw;
        let dialect = self.sanitizer.dialect;
        let html = self.html();
        let mut reader = NsReader::from_str(raw);
        configure(reader.config_mut(), dialect);
        // quick-xml reports positions past a leading BOM, which is copied
        // ahead of the document.
        let mut spans = EventSpans::new(raw);
        self.out.push_str(&raw[..spans.bom()]);

        loop {
            let (namespace, event) = reader.read_resolved_event().map_err(malformed_markup)?;
            let in_xinclude = match namespace {
                ResolveResult::Bound(Namespace(uri)) => is_xinclude(uri)?,
                _ => false,
            };
            let span = spans.next(reader.buffer_position());
            // An end tag's name, lowercased, read once for every check of it.
            let end = match &event {
                Event::End(e) => Some(tag_name(e, raw, dialect)?),
                _ => None,
            };
            let end = end.as_deref();

            if let Some(region) = self.dropping.as_mut() {
                match region.skip(&event, end, raw, dialect)? {
                    Skip::Inside => continue,
                    Skip::Last => {
                        self.dropping = None;
                        continue;
                    }
                    Skip::Outside => self.dropping = None,
                    Skip::End => break,
                }
            }
            match self.text_body(&event, end, span.clone())? {
                Flow::Rebuild => {}
                Flow::Next => continue,
                Flow::Stop => break,
            }
            match &event {
                Event::Eof => break,
                Event::Text(text) => {
                    if let Some(sheet) = self.style.as_mut() {
                        sheet.css.push_str(text.as_ref());
                    }
                    if html {
                        self.copy_html_text(span);
                    } else {
                        self.copy_xml_text(span);
                    }
                }
                Event::GeneralRef(_) => {
                    // A style sheet in SVG or MathML is markup, whose
                    // references a browser decodes before reading the CSS.
                    if let Some(sheet) = self.style.as_mut() {
                        sheet
                            .css
                            .push_str(&decoded(&raw[span.clone()], CharacterReferences::Html));
                    }
                    self.copy(span);
                }
                Event::CData(text) => {
                    // A browser reads a CDATA section as one only in an SVG
                    // or MathML element; in HTML it is a comment.
                    if html && !self.in_foreign_element() {
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
                Event::Start(e) | Event::Empty(e) if html => self.html_start(e)?,
                Event::Start(e) => self.xml_start(e, span, true, in_xinclude)?,
                Event::Empty(e) => self.xml_start(e, span, false, in_xinclude)?,
                // `end` names every end tag.
                Event::End(e) if html => {
                    self.html_end(e.html5_tag(raw)?.name, end.unwrap_or_default())?;
                }
                Event::End(_) => {
                    self.depth = self.depth.saturating_sub(1);
                    self.copy(span);
                }
            }
        }
        // A style sheet left open by lenient HTML runs to the end.
        self.close_style();
        Ok(())
    }

    /// Read `event`, read at `span` and ending the element `end` if an end
    /// tag, against the HTML text body open, whose text a browser reads up to
    /// its own end tag, so markup this reader sees in it would be rebuilt as
    /// what it is not:
    ///
    /// - with script on, a `<noscript>` body runs to its first `</noscript`,
    ///   which markup copied as written must not hold, nor a style sheet read
    ///   as text;
    /// - a `<style>` body is copied as the text it is (see
    ///   [`style_text`](Self::style_text));
    /// - a construct a browser ends before quick-xml does hides live markup
    ///   inside it from this reader;
    /// - a `<textarea>` or `<title>` body holds no markup.
    fn text_body(
        &mut self,
        event: &Event<'_>,
        end: Option<&str>,
        span: Range<usize>,
    ) -> Result<Flow> {
        if self.noscript {
            if !self.in_raw_style() && end == Some("noscript") {
                self.noscript = false;
            } else {
                check_text_end(&self.raw[span.clone()], "noscript")?;
            }
        }
        if self.in_raw_style() {
            return self.style_text(event, end, span);
        }
        if self.html()
            && let Some(construct) = ends_early(event, self.in_foreign_element())
        {
            return Err(Error::malformed(format!(
                "a browser ends this HTML {construct} at an earlier `>`, so markup inside it renders"
            )));
        }
        if let Some(element) = self.rcdata {
            match event {
                Event::Text(_) | Event::GeneralRef(_) | Event::Eof => {}
                Event::End(_) if end == Some(element) => self.rcdata = None,
                _ => {
                    return Err(Error::malformed(format!(
                        "a browser reads the markup inside this HTML <{element}> as its text"
                    )));
                }
            }
        }
        Ok(Flow::Rebuild)
    }

    /// Rebuild the XML start tag `e`, read at `span`, which `is_start` when it
    /// is no empty-element tag: an XInclude goes with what it holds.
    fn xml_start(
        &mut self,
        e: &BytesStart<'_>,
        span: Range<usize>,
        is_start: bool,
        in_xinclude: bool,
    ) -> Result<()> {
        if !in_xinclude {
            self.depth += usize::from(is_start);
            self.copy(span);
            return Ok(());
        }
        if self.depth == 0 {
            return Err(Error::malformed(
                "the XML document element is an XInclude, which leaves no document",
            ));
        }
        self.record(HiddenContent::Link);
        if is_start {
            // XML has one vocabulary, which the region never reads.
            let name = tag_name(e, self.raw, Dialect::Xml)?;
            self.dropping = Some(Region::new(name, Space::Html));
        }
        Ok(())
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

    /// Copy the HTML text at `span`. Where a removal leaves a `&` with the
    /// letters, digits, or `#` after it, which the text would continue into
    /// a reference, those are written as the text a browser reads them as,
    /// escaped, so no reference spans the removal.
    fn copy_html_text(&mut self, span: Range<usize>) {
        let text = &self.raw[span];
        if let Some((amp, len)) = self.dangling
            && len == self.out.len()
            && text.starts_with(|c: char| c.is_ascii_alphanumeric() || matches!(c, '#' | ';'))
        {
            let reads = html::text_reference(&self.out[amp..]);
            let escaped = reads.replace('&', "&amp;").replace('<', "&lt;");
            self.out.replace_range(amp.., &escaped);
        }
        let start = self.out.len();
        self.out.push_str(text);
        let run = text
            .bytes()
            .rev()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'#')
            .count();
        self.dangling = text[..text.len() - run]
            .ends_with('&')
            .then(|| (start + text.len() - run - 1, self.out.len()));
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

    /// Copy `event`, read at `span` and ending the element `end` if an end
    /// tag, inside an HTML `<style>`, whose body a browser reads as text, up
    /// to its end tag.
    fn style_text(
        &mut self,
        event: &Event<'_>,
        end: Option<&str>,
        span: Range<usize>,
    ) -> Result<Flow> {
        match event {
            Event::Eof => return Ok(Flow::Stop),
            Event::End(_) if end == Some("style") => {
                self.copy(span);
                self.close_style();
                return Ok(Flow::Next);
            }
            // The body is raw text, read as written: a comment, a tag or a
            // reference in it is text too, but for a `</style` in it, which a
            // browser ends the body at.
            _ => {
                check_text_end(&self.raw[span.clone()], "style")?;
                if let Some(sheet) = self.style.as_mut() {
                    sheet.css.push_str(&self.raw[span.clone()]);
                }
            }
        }
        self.copy(span);
        Ok(Flow::Next)
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

    /// Rebuild the HTML start tag `e`.
    fn html_start(&mut self, e: &BytesStart<'_>) -> Result<()> {
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
                if !foreign.exact {
                    return Err(unsure_foreign());
                }
                Space::Math
            }
            _ => self.space(),
        };
        // A start tag that implies the end of the HTML element open right in
        // the element read as HTML closes it, kept or not; one that may close
        // more leaves what is open there unsure.
        if space == Space::Html
            && let Some(foreign) = self.spaces.last_mut()
            && foreign.space == Space::Html
        {
            close_implied(&mut foreign.open, &name);
            if html::may_close_more(&name, &foreign.open) {
                foreign.exact = false;
            }
        }
        let element = html::element(space, &name, &tag.attributes, self.sanitizer.policy);
        // The vocabulary the element's content is read in, and its
        // attributes: the root of SVG or MathML enters its own, kept or not.
        let own = match space {
            Space::Html => html::enters(&name).unwrap_or(Space::Html),
            _ => space,
        };
        let opens = html::opens(own, &name, tag.self_closing);
        if let Element::Drop(content) = element {
            self.drop_start(content, name, own, opens, left);
            return Ok(());
        }
        let style_start = self.write_start_tag(&tag, &name, own);
        if opens {
            self.open_kept(element, name, space, style_start)?;
        }
        Ok(())
    }

    /// Leave out, as `content`, the HTML element `name` whose content is read
    /// in `own`, with what it holds when it `opens`; `left` is whether its
    /// start tag left SVG or MathML.
    fn drop_start(
        &mut self,
        content: HiddenContent,
        name: String,
        own: Space,
        opens: bool,
        left: bool,
    ) {
        self.record(content);
        if left {
            // The output leaves SVG or MathML where the source does, so a
            // rebuild of it reads what follows the same way.
            self.out.push_str("<span></span>");
        }
        if opens {
            // A root's content is read in its own vocabulary, so an HTML
            // element in it ends it as it would end the kept root.
            let mut region = Region::new(name, own);
            if own == Space::Html {
                region.text = html::text_element(&region.name);
            }
            self.dropping = Some(region);
        }
    }

    /// Write the start tag `tag` of the kept element `name`, whose attributes
    /// are read in `own`, with the attributes it keeps. Returns where it
    /// starts in the output.
    fn write_start_tag(&mut self, tag: &HtmlTag<'_>, name: &str, own: Space) -> usize {
        let start = self.out.len();
        self.out.push('<');
        self.out.push_str(tag.name);
        let mut seen = HashSet::with_capacity(tag.attributes.len());
        let mut last: Option<&HtmlAttribute<'_>> = None;
        for attr in &tag.attributes {
            let key = attr.name.to_ascii_lowercase();
            // A browser reads the first of a repeated attribute and ignores
            // the rest, which still carry their value, even when the first is
            // left out.
            let content = if seen.contains(&key) {
                Some(HiddenContent::Undisplayed)
            } else {
                let content = html::attribute(own, name, &key, attr.value);
                seen.insert(key);
                content
            };
            if let Some(content) = content {
                self.record(content);
                continue;
            }
            self.out.push(' ');
            self.out.push_str(attr.source);
            last = Some(attr);
        }
        if tag.self_closing {
            // An unquoted value would read a `/` right after it as its own;
            // a quoted one starts right after its quote.
            let unquoted = last.is_some_and(|attr| {
                attr.value.is_some_and(|value| {
                    attr.source
                        .substr_range(value)
                        .and_then(|range| range.start.checked_sub(1))
                        .is_none_or(|quote| !matches!(attr.source.as_bytes()[quote], b'"' | b'\''))
                })
            });
            self.out.push_str(if unquoted { " />" } else { "/>" });
        } else {
            self.out.push('>');
        }
        start
    }

    /// Open the kept `element` named `name`, read in `space`, whose start tag
    /// begins at `style_start` in the output.
    fn open_kept(
        &mut self,
        element: Element,
        name: String,
        space: Space,
        style_start: usize,
    ) -> Result<()> {
        match element {
            Element::Enter(entered) => self.spaces.push(Foreign::new(entered, name.clone())),
            // Inside an SVG `desc` or a MathML `mi`, a browser reads HTML
            // until the element ends.
            _ if html::integration_point(space, &name) => {
                self.spaces.push(Foreign::new(Space::Html, name.clone()));
            }
            _ => {
                if let Some(foreign) = self.spaces.last_mut() {
                    // An `mglyph` in a token element is MathML, which ends
                    // by other rules than the HTML around it.
                    if foreign.space == Space::Html && space != Space::Html {
                        foreign.exact = false;
                    }
                    foreign.open.push(name.clone(), ());
                }
            }
        }
        if space == Space::Html && name == "noscript" {
            self.noscript = true;
        }
        if space == Space::Html
            && let Some(element) = html::rcdata_element(&name)
        {
            self.rcdata = Some(element);
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
                as_text: space == Space::Html,
            });
        }
        Ok(())
    }

    /// Rebuild the HTML end tag `written`, named `name` lowercased.
    fn html_end(&mut self, written: &str, name: &str) -> Result<()> {
        let space = self.space();
        // An end tag holds nothing, so one the allowlist does not name is
        // left out unrecorded; a stray HTML one in SVG or MathML still acts.
        let named = html::names(space, name) || html::names(Space::Html, name);
        let closed = self.end_foreign(name)?;
        // A `style` is always named.
        if named || closed {
            self.out.push_str("</");
            self.out.push_str(written);
            self.out.push('>');
        }
        // A style sheet in SVG ends with the element it is.
        if self.style.as_ref().is_some_and(|sheet| !sheet.as_text)
            && !self
                .spaces
                .iter()
                .any(|foreign| foreign.space != Space::Html && foreign.open.contains("style"))
        {
            self.close_style();
        }
        Ok(())
    }

    /// Close what the end tag `name` (lowercased) closes of the SVG and
    /// MathML open and the HTML read inside them. Returns whether it closed
    /// an SVG or MathML element.
    ///
    /// Where an HTML element is open, a browser reads it by HTML's rules,
    /// which stop at the element reading HTML. Where an SVG or MathML element
    /// is, even one reading HTML, it closes the innermost of them of its name
    /// with everything inside it, and failing one, falls to HTML's rules from
    /// the first HTML element; those stop at an element reading HTML passed on
    /// the way.
    fn end_foreign(&mut self, name: &str) -> Result<bool> {
        let Some(last) = self.spaces.last_mut() else {
            return Ok(false);
        };
        if last.space == Space::Html && !last.open.is_empty() {
            if last.open.contains(name) {
                let above = last.open.innermost_first().map(|(open, ())| open.as_str());
                if !html::closes_cleanly(name, above.take_while(|open| *open != name)) {
                    last.exact = false;
                }
                last.open.close(name);
            } else if !last.exact && self.names_foreign(name) {
                return Err(unsure_foreign());
            }
            return Ok(false);
        }
        // `</br>` and `</p>` leave SVG and MathML, as an HTML start tag does,
        // and are read as HTML.
        if last.space != Space::Html && matches!(name, "br" | "p") {
            self.leave_foreign();
            return Ok(true);
        }
        let mut passed_point = false;
        for at in (0..self.spaces.len()).rev() {
            let foreign = &mut self.spaces[at];
            if foreign.space == Space::Html {
                if !foreign.exact {
                    return Err(unsure_foreign());
                }
                if !foreign.open.is_empty() {
                    // An HTML element, past SVG or MathML elements.
                    if !passed_point && foreign.open.contains(name) {
                        return Err(unsure_foreign());
                    }
                    return Ok(false);
                }
                if foreign.root == name {
                    self.spaces.truncate(at);
                    return Ok(true);
                }
                passed_point = true;
            } else if foreign.open.close(name) {
                self.spaces.truncate(at + 1);
                return Ok(true);
            } else if foreign.root == name {
                self.spaces.truncate(at);
                return Ok(true);
            }
        }
        // The HTML around them, which this reader does not follow: an end tag
        // that may close an element of it closes the SVG or MathML too.
        if passed_point || matches!(name, "html" | "body") {
            return Ok(false);
        }
        Err(unsure_foreign())
    }

    /// Whether `name` (lowercased) names an SVG or MathML element open.
    fn names_foreign(&self, name: &str) -> bool {
        self.spaces.iter().any(|foreign| {
            foreign.root == name || (foreign.space != Space::Html && foreign.open.contains(name))
        })
    }
}

/// The refusal of markup whose SVG or MathML a browser may close where this
/// reader does not.
fn unsure_foreign() -> Error {
    Error::malformed(
        "a browser may close the SVG or MathML here otherwise than this reader, \
         reading what follows in another vocabulary",
    )
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
/// vocabulary its content is read in, and the elements open inside it, so its
/// own end tag is told from theirs.
struct Region {
    name: String,
    space: Space,
    open: OpenElements<Space>,
    /// The text element, open in it or itself, whose body a browser reads
    /// as text up to its end tag.
    text: Option<&'static str>,
    /// Whether end tags close what they name, as read when `open` had seen
    /// this many changes.
    closes_read: (u64, HashMap<String, bool>),
}

impl Region {
    fn new(name: String, space: Space) -> Self {
        Self {
            name,
            space,
            open: OpenElements::new(),
            text: None,
            closes_read: (0, HashMap::new()),
        }
    }

    /// The element whose content is being read, with its vocabulary: the
    /// innermost one open in it, else itself.
    fn current(&self) -> (&str, Space) {
        self.open
            .last()
            .map_or((self.name.as_str(), self.space), |(name, space)| {
                (name.as_str(), *space)
            })
    }

    /// Follow `event`, read from `raw` as `dialect` reads it and ending the
    /// element `end` if an end tag, inside the element, which ends at its own
    /// end tag past the elements opened inside it, or where a browser ends it
    /// without one:
    ///
    /// - in HTML, at a start tag that closes it when nothing inside it is
    ///   open (`<p hidden>a<p>b` closes the first paragraph);
    /// - in SVG or MathML, at an element that breaks out to HTML, unless an
    ///   element inside it reads its content as HTML.
    ///
    /// Its own end tag does not end it past an element open in it that keeps
    /// a browser from reading the tag as its end.
    fn skip(
        &mut self,
        event: &Event<'_>,
        end: Option<&str>,
        raw: &str,
        dialect: Dialect,
    ) -> Result<Skip> {
        // Inside a text element, only its own end tag is markup.
        if let Some(text) = self.text {
            let ends = text != "plaintext" && end == Some(text);
            if !ends && !matches!(event, Event::Eof) {
                return Ok(Skip::Inside);
            }
            self.text = None;
        }
        match event {
            // An element left open by lenient HTML runs to the end of the
            // document.
            Event::Eof => Ok(Skip::End),
            Event::Start(e) | Event::Empty(e) if dialect == Dialect::Html => {
                let tag = e.html5_tag(raw)?;
                let name = tag.name.to_ascii_lowercase();
                if html::breaks_out(self.current().1, &name, &tag.attributes) && self.break_out() {
                    return Ok(Skip::Outside);
                }
                let space = self.child_space(&name);
                if space == Space::Html && self.implied_end(&name) {
                    return Ok(Skip::Outside);
                }
                let opens = html::opens(space, &name, tag.self_closing);
                self.open_child(name, space, opens);
                Ok(Skip::Inside)
            }
            Event::Start(e) => {
                let space = self.space;
                self.open.push(tag_name(e, raw, Dialect::Xml)?, space);
                Ok(Skip::Inside)
            }
            Event::End(_) => {
                let name = end.unwrap_or_default();
                let inside = self.open.contains(name);
                if !inside && name != self.name || !self.closes(name, inside) {
                    return Ok(Skip::Inside);
                }
                if inside {
                    self.open.close(name);
                    return Ok(Skip::Inside);
                }
                Ok(Skip::Last)
            }
            _ => Ok(Skip::Inside),
        }
    }

    /// Whether the end tag `name` (lowercased) closes the innermost element
    /// of its name open `inside` it, else the element itself, past what is
    /// open in it: unless that keeps a browser from reading the tag as its
    /// end, which it then ignores.
    fn closes(&mut self, name: &str, inside: bool) -> bool {
        // Known while nothing has opened or closed since it was read.
        let changes = self.open.changes();
        if self.closes_read.0 != changes {
            self.closes_read = (changes, HashMap::new());
        }
        if let Some(closes) = self.closes_read.1.get(name) {
            return *closes;
        }
        let mut target = self.space;
        let mut above = Vec::new();
        for (open, space) in self.open.innermost_first() {
            if inside && open == name {
                target = *space;
                break;
            }
            above.push((open.as_str(), *space));
        }
        let closes = !html::keeps_open(name, target, above);
        self.closes_read.1.insert(name.to_owned(), closes);
        closes
    }

    /// Close what an HTML element breaking out of SVG or MathML closes: the
    /// foreign elements open in it, up to one that reads its content as HTML.
    /// Returns whether that ends the element itself.
    fn break_out(&mut self) -> bool {
        while let Some((open, space)) = self.open.last()
            && *space != Space::Html
            && !html::integration_point(*space, open)
        {
            self.open.pop();
        }
        self.open.is_empty()
            && self.space != Space::Html
            && !html::integration_point(self.space, &self.name)
    }

    /// The vocabulary the element `name` (lowercased) started in it reads its
    /// content in: HTML where the current element reads its content as HTML,
    /// but for an `svg` or `math` root; else the current element's, but for
    /// an `svg` in a MathML `annotation-xml`.
    fn child_space(&self, name: &str) -> Space {
        let (current, current_space) = self.current();
        let as_html = current_space == Space::Html
            || (html::integration_point(current_space, current)
                && !(current_space == Space::Math && html::stays_math(name)));
        if as_html {
            html::enters(name).unwrap_or(Space::Html)
        } else if name == "svg" && current == "annotation-xml" {
            Space::Svg
        } else {
            current_space
        }
    }

    /// Close the elements open in it whose end the HTML start tag `name`
    /// (lowercased) implies, void or self-closed as it may be. Returns
    /// whether that leaves nothing open and `name` implies the element's own
    /// end too (`<p hidden>a<p>b`).
    fn implied_end(&mut self, name: &str) -> bool {
        close_implied(&mut self.open, name);
        self.space == Space::Html && self.open.is_empty() && html::closes(&self.name, name)
    }

    /// Open the element `name`, whose content is read in `space`, in it when
    /// it `opens`; an HTML text element's body is then read as text.
    fn open_child(&mut self, name: String, space: Space, opens: bool) {
        if !opens {
            return;
        }
        if space == Space::Html
            && let Some(text) = html::text_element(&name)
        {
            self.text = Some(text);
        }
        self.open.push(name, space);
    }
}

/// A `<style>` element being read: where its start tag begins in the output,
/// its text so far, and whether a browser reads its body `as_text` (in HTML)
/// or as markup (in SVG).
struct StyleSheet {
    start: usize,
    css: String,
    as_text: bool,
}

/// SVG or MathML entered by its root element, or HTML read inside an element
/// of theirs, and the elements open in it.
struct Foreign {
    space: Space,
    root: String,
    /// The elements open inside the root, innermost last: SVG or MathML, or
    /// under HTML read inside an element, HTML (and an `mglyph`), so what is
    /// started right in that element is told apart.
    open: OpenElements,
    /// Under HTML read inside an element: whether `open` is what a browser
    /// has open, which it is not once a tag may have closed otherwise.
    exact: bool,
}

impl Foreign {
    fn new(space: Space, root: String) -> Self {
        Self {
            space,
            root,
            open: OpenElements::new(),
            exact: true,
        }
    }
}

/// The construct `event` is, when a browser ends it before quick-xml does, so
/// markup quick-xml reads as inside it renders: a comment a browser closes
/// abruptly (`<!-->`, `<!--->`) or at `--!>`, and a processing instruction,
/// CDATA section (but `in_foreign` an SVG or MathML element, where it is one),
/// or document type with a `>` inside, which a browser ends at its first `>`.
fn ends_early(event: &Event<'_>, in_foreign: bool) -> Option<&'static str> {
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
        Event::CData(text) => {
            (!in_foreign && text.as_ref().contains('>')).then_some("CDATA section")
        }
        Event::DocType(text) => text.as_ref().contains('>').then_some("document type"),
        _ => None,
    }
}

/// Refuse the `text` of the HTML element `name` (lowercased), read as text,
/// when it holds an end tag a browser ends that body at.
fn check_text_end(text: &str, name: &str) -> Result<()> {
    if ends_text(text, name) {
        return Err(Error::malformed(format!(
            "a browser ends this HTML <{name}> at a `</{name}` inside markup, so what follows it renders"
        )));
    }
    Ok(())
}

/// Whether `text` holds an end tag for the element `name` (lowercased), in
/// any case: its name followed by HTML whitespace, a `/`, a `>`, or the end
/// of `text`.
fn ends_text(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();
    let len = name.len() + 2;
    bytes.windows(len).enumerate().any(|(at, window)| {
        window.starts_with(b"</")
            && window[2..].eq_ignore_ascii_case(name.as_bytes())
            && bytes
                .get(at + len)
                .is_none_or(|b| matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ' | b'/' | b'>'))
    })
}

/// Whether the namespace `uri`, as written, is XInclude's once its character
/// references are decoded.
fn is_xinclude(uri: &str) -> Result<bool> {
    if !uri.contains('&') {
        return Ok(uri == XINCLUDE);
    }
    let decoded = quick_xml::escape::unescape(uri)
        .map_err(|e| malformed_markup(format_args!("namespace {e}")))?;
    Ok(decoded == XINCLUDE)
}

/// Whether a comment's `text` is a conditional comment (`[if IE]>…`), markup
/// an old browser parses and runs.
fn is_conditional(text: &str) -> bool {
    text.trim_start().starts_with("[if")
}

#[cfg(test)]
mod tests;
