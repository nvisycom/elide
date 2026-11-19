//! [`edit`]: rewrite an XML part's elements and attributes by namespace-aware
//! [`Rules`], leaving every byte outside an edit as it was.
//!
//! An OOXML part is sanitized by cutting byte ranges out of its source: an
//! element removed with its subtree, an element unwrapped (its tags gone, its
//! children kept), an attribute removed from a start tag, or an attribute's
//! value replaced with a neutral one. Nothing is
//! re-serialized, so formatting, namespace declarations, and every untouched
//! element round-trip exactly. Element and attribute names are matched by
//! namespace URI, never by prefix, so a part that binds `w:` to an unusual
//! prefix is still recognized.

use std::borrow::Cow;
use std::fmt::Display;
use std::ops::Range;

use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use quick_xml::escape::unescape;
use quick_xml::events::attributes::Attribute as XmlAttribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::{NsReader, Reader};

use crate::opc::malformed_xml;

/// A namespace-qualified element or attribute name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Name<'a> {
    /// The bound namespace URI, or `None` when the name is in no namespace.
    pub(crate) namespace: Option<&'a str>,
    /// The local name.
    pub(crate) local: &'a str,
}

impl Name<'_> {
    /// Whether this is `local` in `namespace`.
    pub(crate) fn is(&self, namespace: &str, local: &str) -> bool {
        self.namespace == Some(namespace) && self.local == local
    }

    /// Whether this is in `namespace`.
    pub(crate) fn in_namespace(&self, namespace: &str) -> bool {
        self.namespace == Some(namespace)
    }
}

/// One attribute of a start tag: its name and its value as written (still
/// escaped).
#[derive(Debug, Clone)]
pub(crate) struct Attribute<'a> {
    pub(crate) name: Name<'a>,
    pub(crate) value: &'a str,
    /// The attribute's byte range in the part, its leading whitespace included.
    range: Range<usize>,
    /// The byte range of its value, inside the quotes.
    value_range: Range<usize>,
}

/// What an edit does with an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Keep the element (its attributes may still be removed).
    Keep,
    /// Remove the element with everything inside it.
    Remove,
    /// Remove the element's own tags, keeping its children in its place.
    Unwrap,
}

/// The decisions an [`edit`] asks of a format, for each element it meets.
pub(crate) trait Rules {
    /// The hidden content every removal these rules make is, or `None` for
    /// rules whose removals are not hidden content of their own (a cascade
    /// that follows another removal, or a check that removes nothing).
    fn content(&self) -> Option<HiddenContent>;

    /// What to do with the element `name` carrying `attributes`.
    fn element(&self, name: Name<'_>, attributes: &[Attribute<'_>]) -> Action;

    /// Whether to remove `attribute` from a kept element `element`.
    fn removes_attribute(&self, _element: Name<'_>, _attribute: &Attribute<'_>) -> bool {
        false
    }

    /// The neutral value to give `attribute` of a kept element `element`, for
    /// one that cannot be removed. An attribute already holding it is left as
    /// it is, so a neutralized part edits to nothing.
    fn neutralizes_attribute(
        &self,
        _element: Name<'_>,
        _attribute: &Attribute<'_>,
    ) -> Option<&'static str> {
        None
    }
}

/// One edit of an XML part, naming the hidden content it removes. Every
/// [`Rules`] is one, applied by [`edit`]; an edit that must see inside an
/// element before cutting it implements this directly.
pub(crate) trait Edit {
    /// The hidden content every removal this edit makes is, or `None` (as
    /// [`Rules::content`]).
    fn content(&self) -> Option<HiddenContent>;

    /// `raw` edited, or `None` when the edit changes nothing.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if `raw` is not well-formed XML.
    fn apply(&self, raw: &str) -> Result<Option<Edited>>;
}

/// The XML comments of a part. Nothing in a package reads one and no reader of
/// the document sees it, so it goes whatever the policy.
pub(crate) struct XmlComments;

impl Edit for XmlComments {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Undisplayed)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        if !raw.contains("<!--") {
            return Ok(None);
        }
        let mut reader = SpannedReader::new(raw);
        let mut cuts = Vec::new();
        loop {
            let (event, span) = reader.read_event().map_err(malformed_xml)?;
            match event {
                Event::Eof => break,
                Event::Comment(_) => cuts.push((span, "")),
                _ => {}
            }
        }
        if cuts.is_empty() {
            return Ok(None);
        }
        let count = cuts.len();
        Ok(Some(Edited {
            text: splice(raw, cuts),
            count,
        }))
    }
}

impl<R: Rules> Edit for R {
    fn content(&self) -> Option<HiddenContent> {
        Rules::content(self)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        edit(raw, self)
    }
}

/// A namespace-qualified name to match: `(namespace URI, local name)`.
pub(crate) type QName = (&'static str, &'static str);

/// How an attribute's local name is matched.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Local {
    /// Exactly this local name.
    Exact(&'static str),
    /// Any local name starting with this prefix.
    Prefix(&'static str),
    /// Any local name.
    Any,
}

impl Local {
    /// Whether `local` is matched.
    pub(crate) fn matches(self, local: &str) -> bool {
        match self {
            Self::Exact(exact) => local == exact,
            Self::Prefix(prefix) => local.starts_with(prefix),
            Self::Any => true,
        }
    }
}

/// [`Rules`] declared as name lists: elements to remove, elements to unwrap,
/// and attributes (by namespace and local-name match) to remove from every
/// element kept, all of them the one item of hidden content `content`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Declared {
    pub(crate) content: HiddenContent,
    pub(crate) remove: &'static [QName],
    pub(crate) unwrap: &'static [QName],
    pub(crate) attributes: &'static [(&'static str, Local)],
}

impl Declared {
    /// Rules removing the elements `remove`, the hidden content `content`.
    pub(crate) const fn removing(content: HiddenContent, remove: &'static [QName]) -> Self {
        Self {
            content,
            remove,
            unwrap: &[],
            attributes: &[],
        }
    }

    /// These rules, also removing the attributes `attributes` from every
    /// element kept.
    pub(crate) const fn with_attributes(
        mut self,
        attributes: &'static [(&'static str, Local)],
    ) -> Self {
        self.attributes = attributes;
        self
    }
}

impl Rules for Declared {
    fn content(&self) -> Option<HiddenContent> {
        Some(self.content)
    }

    fn element(&self, name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
        let matches = |list: &[QName]| {
            list.iter()
                .any(|&(namespace, local)| name.is(namespace, local))
        };
        if matches(self.remove) {
            Action::Remove
        } else if matches(self.unwrap) {
            Action::Unwrap
        } else {
            Action::Keep
        }
    }

    fn removes_attribute(&self, _element: Name<'_>, attribute: &Attribute<'_>) -> bool {
        self.attributes.iter().any(|&(namespace, local)| {
            attribute.name.in_namespace(namespace) && local.matches(attribute.name.local)
        })
    }
}

/// How deep elements may nest before a part is refused. Real OOXML nests a few
/// dozen levels at most (a table in a text box in a table); bounding it keeps
/// every reader of the part, ours and the consumer's, within a bounded stack.
const MAX_DEPTH: usize = 256;

/// Check that `raw` is a part this crate can read safely: well-formed, nested
/// within [`MAX_DEPTH`], and with no document type declaration.
///
/// # Errors
///
/// As [`edit`].
pub(crate) fn check(raw: &str) -> Result<()> {
    edit(raw, &Unchanged).map(|_| ())
}

/// Rules that change nothing, for a structural [`check`].
struct Unchanged;

impl Rules for Unchanged {
    fn content(&self) -> Option<HiddenContent> {
        None
    }

    fn element(&self, _name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
        Action::Keep
    }
}

/// A part edited by [`edit`]: its new text, and how many elements and
/// attributes the edit removed or unwrapped.
#[derive(Debug)]
pub(crate) struct Edited {
    pub(crate) text: String,
    pub(crate) count: usize,
}

/// `raw` as `edit` changes it, for a test whose edit changes something.
///
/// # Panics
///
/// When the edit fails or changes nothing.
#[cfg(test)]
pub(crate) fn expect_edit(raw: &str, edit: &(impl Edit + ?Sized)) -> Edited {
    edit.apply(raw)
        .unwrap()
        .expect("the edit changes the document")
}

/// `raw` edited by `rules`, or `None` when the rules change nothing.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if `raw` is not well-formed XML, nests past
/// [`MAX_DEPTH`], or declares a document type: a DTD's entities expand into
/// text no extraction reads, and OOXML never uses one.
pub(crate) fn edit(raw: &str, rules: &dyn Rules) -> Result<Option<Edited>> {
    let mut reader = SpannedReader::new_ns(raw);
    // Each byte range cut, and what takes its place (nothing, or a neutral
    // attribute value).
    let mut cuts: Vec<(Range<usize>, &str)> = Vec::new();
    // Elements and attributes removed, unwrapped, or neutralized; an unwrapped
    // element's end tag is cut too, but it is one element.
    let mut count = 0;
    // The open elements' actions, to know what an end tag closes.
    let mut open: Vec<Action> = Vec::new();
    // A removed element in progress.
    let mut removing: Option<Skipped> = None;

    loop {
        let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;

        if let Some(skipped) = &mut removing {
            if let Some(whole) = skipped.read(&event, span.end)? {
                cuts.push((whole, ""));
                removing = None;
            }
            continue;
        }

        match event {
            Event::Eof => break,
            Event::DocType(_) => {
                return Err(malformed_xml("a document type declaration"));
            }
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_start = matches!(event, Event::Start(_));
                if is_start && open.len() >= MAX_DEPTH {
                    return Err(malformed_xml("elements nest too deep"));
                }
                let local = e.local_name();
                let name = Name {
                    namespace: namespace.as_deref(),
                    local: local.into_inner(),
                };
                let owned = attributes(&reader.reader, raw, e)?;
                let attributes: Vec<Attribute<'_>> =
                    owned.iter().map(OwnedAttribute::view).collect();
                match rules.element(name, &attributes) {
                    Action::Remove if is_start => {
                        removing = Some(Skipped::new(span.start));
                        count += 1;
                    }
                    Action::Remove | Action::Unwrap => {
                        cuts.push((span.clone(), ""));
                        count += 1;
                        if is_start {
                            open.push(Action::Unwrap);
                        }
                    }
                    Action::Keep => {
                        for attribute in &attributes {
                            if rules.removes_attribute(name, attribute) {
                                cuts.push((attribute.range.clone(), ""));
                                count += 1;
                            } else if let Some(neutral) =
                                rules.neutralizes_attribute(name, attribute)
                                && attribute.value != neutral
                            {
                                cuts.push((attribute.value_range.clone(), neutral));
                                count += 1;
                            }
                        }
                        if is_start {
                            open.push(Action::Keep);
                        }
                    }
                }
            }
            Event::End(_) => {
                let closed = open.pop();
                if closed == Some(Action::Unwrap) {
                    cuts.push((span, ""));
                }
            }
            _ => {}
        }
    }

    if cuts.is_empty() {
        return Ok(None);
    }
    // An attribute cut never overlaps another cut: an element removed whole is
    // cut before its attributes are looked at.
    Ok(Some(Edited {
        text: splice(raw, cuts),
        count,
    }))
}

/// An attribute with its namespace resolved into an owned string, so it
/// outlives the reader borrow the resolution took.
struct OwnedAttribute<'a> {
    namespace: Option<String>,
    local: &'a str,
    value: &'a str,
    range: Range<usize>,
    value_range: Range<usize>,
}

impl OwnedAttribute<'_> {
    fn view(&self) -> Attribute<'_> {
        Attribute {
            name: Name {
                namespace: self.namespace.as_deref(),
                local: self.local,
            },
            value: self.value,
            range: self.range.clone(),
            value_range: self.value_range.clone(),
        }
    }
}

/// The namespace URI a resolved name is bound to, if any.
pub(crate) fn bound(resolved: &ResolveResult<'_>) -> Option<String> {
    match resolved {
        ResolveResult::Bound(Namespace(uri)) => Some((*uri).to_owned()),
        _ => None,
    }
}

/// Every attribute of `e`, resolved and located in `raw`.
fn attributes<'a>(
    reader: &NsReader<&[u8]>,
    raw: &'a str,
    e: &BytesStart<'a>,
) -> Result<Vec<OwnedAttribute<'a>>> {
    let mut out = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(malformed_xml)?;
        let (key, value) = attribute_source(raw, &attr)?;
        let (resolved, local) = reader.resolver().resolve_attribute(attr.key);
        let start = raw[..key.start]
            .trim_end_matches(|c: char| c.is_ascii_whitespace())
            .len();
        out.push(OwnedAttribute {
            namespace: bound(&resolved),
            local: &raw[key.end - local.into_inner().len()..key.end],
            value: &raw[value.clone()],
            // Past the closing quote.
            range: start..value.end + 1,
            value_range: value,
        });
    }
    Ok(out)
}

/// The byte length of a leading UTF-8 BOM (`U+FEFF`), or 0 if absent. quick-xml
/// skips it and reports positions past it, so spans over the original bytes must
/// add it back.
pub(crate) fn bom_len(raw: &str) -> usize {
    if raw.starts_with('\u{feff}') { 3 } else { 0 }
}

/// A quick-xml reader over a part that gives each event with its byte span in
/// the part's own bytes, a leading BOM included.
pub(crate) struct SpannedReader<R> {
    /// The reader, for what else it tells (a namespace resolver).
    pub(crate) reader: R,
    /// The byte length of the part's BOM, which quick-xml's positions leave out.
    bom: usize,
    /// Where the last event read ended.
    last: usize,
}

impl<R> SpannedReader<R> {
    /// `reader` over `raw`.
    fn over(reader: R, raw: &str) -> Self {
        let bom = bom_len(raw);
        Self {
            reader,
            bom,
            last: bom,
        }
    }

    /// The span of the event read up to the reader's `position`.
    fn span(&mut self, position: u64) -> Range<usize> {
        let span = self.last..position as usize + self.bom;
        self.last = span.end;
        span
    }
}

impl<'a> SpannedReader<Reader<&'a [u8]>> {
    /// A reader over `raw`.
    pub(crate) fn new(raw: &'a str) -> Self {
        Self::over(Reader::from_str(raw), raw)
    }

    /// The next event and its span.
    pub(crate) fn read_event(&mut self) -> quick_xml::Result<(Event<'a>, Range<usize>)> {
        let event = self.reader.read_event()?;
        let span = self.span(self.reader.buffer_position());
        Ok((event, span))
    }
}

impl<'a> SpannedReader<NsReader<&'a [u8]>> {
    /// A namespace-aware reader over `raw`.
    pub(crate) fn new_ns(raw: &'a str) -> Self {
        Self::over(NsReader::from_str(raw), raw)
    }

    /// The next event, the namespace URI its name is bound to, and its span.
    pub(crate) fn read_resolved_event(
        &mut self,
    ) -> quick_xml::Result<(Option<String>, Event<'a>, Range<usize>)> {
        let (resolved, event) = self.reader.read_resolved_event()?;
        let namespace = bound(&resolved);
        let span = self.span(self.reader.buffer_position());
        Ok((namespace, event, span))
    }
}

/// An element being skipped with everything inside it, read event by event.
pub(crate) struct Skipped {
    /// Where its start tag begins.
    start: usize,
    /// How many elements are open inside it, itself included.
    depth: usize,
}

impl Skipped {
    /// The element whose start tag begins at `start`.
    pub(crate) fn new(start: usize) -> Self {
        Self { start, depth: 1 }
    }

    /// Read `event`, ending at `end`, inside the element: the element's whole
    /// span once its end tag is read, else `None`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the part ends inside it.
    pub(crate) fn read(&mut self, event: &Event<'_>, end: usize) -> Result<Option<Range<usize>>> {
        match event {
            Event::Start(_) => self.depth += 1,
            Event::End(_) => {
                self.depth -= 1;
                if self.depth == 0 {
                    return Ok(Some(self.start..end));
                }
            }
            Event::Eof => return Err(malformed_xml("unclosed element")),
            _ => {}
        }
        Ok(None)
    }
}

/// `raw` with each of `cuts` replaced by its text, cuts applied in order of
/// where they start (an insertion ahead of a cut starting where it does). A
/// cut may nest in or overlap one before it: only the part past what is
/// already cut goes, and its text is still written.
pub(crate) fn splice<S: AsRef<str>>(raw: &str, cuts: Vec<(Range<usize>, S)>) -> String {
    splice_with(raw, cuts, String::push_str)
}

/// [`splice`], writing each kept span of `raw` with `keep`.
pub(crate) fn splice_with<S: AsRef<str>>(
    raw: &str,
    mut cuts: Vec<(Range<usize>, S)>,
    mut keep: impl FnMut(&mut String, &str),
) -> String {
    cuts.sort_by_key(|(range, _)| (range.start, range.end));
    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0;
    for (cut, with) in cuts {
        keep(&mut out, &raw[cursor..cut.start.max(cursor)]);
        out.push_str(with.as_ref());
        cursor = cursor.max(cut.end);
    }
    keep(&mut out, &raw[cursor..]);
    out
}

/// [`splice`] for cuts that must not overlap, into the part `part`, checked
/// first with [`check_cuts`].
///
/// # Errors
///
/// As [`check_cuts`].
pub(crate) fn splice_checked<S: AsRef<str>>(
    raw: &str,
    part: impl Display,
    mut cuts: Vec<(Range<usize>, S)>,
) -> Result<String> {
    cuts.sort_by_key(|(range, _)| (range.start, range.end));
    check_cuts(raw, part, cuts.iter().map(|(range, _)| range.clone()))?;
    Ok(splice(raw, cuts))
}

/// Check that each of `ranges`, in order of where they start, lies within
/// `raw` on character boundaries and clear of the one before it, for a splice
/// into the part `part`.
///
/// # Errors
///
/// [`ErrorKind::Processing`](elide_core::ErrorKind::Processing) naming the
/// first range that does not.
pub(crate) fn check_cuts(
    raw: &str,
    part: impl Display,
    ranges: impl IntoIterator<Item = Range<usize>>,
) -> Result<()> {
    let mut prev_end = 0;
    for Range { start, end } in ranges {
        if start > end || end > raw.len() {
            return Err(Error::processing(format!(
                "span {start}..{end} out of bounds in `{part}` (len {})",
                raw.len()
            )));
        }
        if !raw.is_char_boundary(start) || !raw.is_char_boundary(end) {
            return Err(Error::processing(format!(
                "span {start}..{end} falls mid-character in `{part}`"
            )));
        }
        if start < prev_end {
            return Err(Error::processing(format!(
                "span {start}..{end} overlaps an earlier one in `{part}`"
            )));
        }
        prev_end = end;
    }
    Ok(())
}

/// Where the attribute `attr`, read from `raw`, is written in it: its name,
/// and its value inside the quotes.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// `attr` was not read straight out of `raw`.
pub(crate) fn attribute_source(
    raw: &str,
    attr: &XmlAttribute<'_>,
) -> Result<(Range<usize>, Range<usize>)> {
    let outside = || malformed_xml("attribute outside the part");
    let key = raw
        .substr_range(attr.key.into_inner())
        .ok_or_else(outside)?;
    let Cow::Borrowed(value) = &attr.value else {
        return Err(malformed_xml("attribute without a source"));
    };
    let value = raw.substr_range(value).ok_or_else(outside)?;
    Ok((Range::from(key), Range::from(value)))
}

/// `xml` without each empty element `local` whose attribute `attribute`,
/// unescaped, `drops` accepts, byte-for-byte otherwise.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput), as
/// `what` malformed, if `xml` is not well-formed.
pub(crate) fn without_elements(
    xml: &str,
    what: &str,
    local: &str,
    attribute: &str,
    drops: impl Fn(&str) -> bool,
) -> Result<String> {
    let mut reader = SpannedReader::new(xml);
    let mut cuts = Vec::new();
    loop {
        let (event, span) = reader
            .read_event()
            .map_err(|e| Error::malformed(format!("{what} malformed: {e}")))?;
        let element = match event {
            Event::Eof => break,
            Event::Empty(e) if e.local_name().as_ref() == local => e,
            _ => continue,
        };
        let value = element
            .attributes()
            .flatten()
            .find(|attr| attr.key.local_name().as_ref() == attribute)
            .and_then(|attr| unescape(&attr.value).ok().map(Cow::into_owned));
        if value.is_some_and(|value| drops(&value)) {
            cuts.push((span, ""));
        }
    }
    Ok(splice(xml, cuts))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    /// Remove `w:del`, unwrap `w:ins`, and drop `w:rsid*` attributes.
    struct Revisions;

    impl Rules for Revisions {
        fn content(&self) -> Option<HiddenContent> {
            Some(HiddenContent::TrackedChange)
        }

        fn element(&self, name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
            if name.is(W, "del") {
                Action::Remove
            } else if name.is(W, "ins") {
                Action::Unwrap
            } else {
                Action::Keep
            }
        }

        fn removes_attribute(&self, _element: Name<'_>, attribute: &Attribute<'_>) -> bool {
            attribute.name.in_namespace(W) && attribute.name.local.starts_with("rsid")
        }
    }

    #[test]
    fn removes_unwraps_and_strips_attributes_byte_faithfully() {
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:rsidR="00AB"><w:ins w:author="Jane"><w:r><w:t>kept</w:t></w:r></w:ins><w:del w:author="Jane"><w:r><w:delText>gone</w:delText></w:r></w:del><w:r w:rsidRPr="1"><w:t xml:space="preserve"> tail</w:t></w:r></w:p>"#;
        let edited = expect_edit(raw, &Revisions);
        // The paragraph's rsid, the insertion, the deletion, the run's rsid.
        assert_eq!(edited.count, 4);
        assert_eq!(
            edited.text,
            r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>kept</w:t></w:r><w:r><w:t xml:space="preserve"> tail</w:t></w:r></w:p>"#
        );
    }

    #[test]
    fn matches_by_namespace_not_prefix() {
        let raw = r#"<x:del xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:r/></x:del><del>kept</del>"#;
        assert_eq!(expect_edit(raw, &Revisions).text, "<del>kept</del>");
    }

    #[test]
    fn an_untouched_part_is_unchanged() {
        assert!(
            edit(r#"<w:p xmlns:w="urn:other"><w:del/></w:p>"#, &Revisions)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn check_refuses_a_document_type_and_runaway_nesting() {
        assert!(check(r#"<!DOCTYPE d [<!ENTITY x "Jane">]><d>&x;</d>"#).is_err());
        let deep = format!(
            "{}{}",
            "<a>".repeat(MAX_DEPTH + 1),
            "</a>".repeat(MAX_DEPTH + 1)
        );
        assert!(check(&deep).is_err());
        assert!(check("<a><b/></a>").is_ok());
    }

    #[test]
    fn spans_index_the_bytes_past_a_byte_order_mark() {
        let raw = "\u{feff}<a>x</a>";
        let mut reader = SpannedReader::new(raw);
        let mut spans = Vec::new();
        loop {
            let (event, span) = reader.read_event().unwrap();
            if matches!(event, Event::Eof) {
                break;
            }
            spans.push(&raw[span]);
        }
        assert_eq!(spans, ["<a>", "x", "</a>"]);
    }

    #[test]
    fn a_skipped_element_spans_its_subtree() {
        let raw = "<a><b><c/></b><d/></a>";
        let mut reader = SpannedReader::new(raw);
        let mut skipped: Option<Skipped> = None;
        loop {
            let (event, span) = reader.read_event().unwrap();
            if let Some(skip) = &mut skipped {
                if let Some(whole) = skip.read(&event, span.end).unwrap() {
                    assert_eq!(&raw[whole], "<b><c/></b>");
                    return;
                }
            } else if matches!(&event, Event::Start(e) if e.local_name().as_ref() == "b") {
                skipped = Some(Skipped::new(span.start));
            }
        }
    }

    #[test]
    fn a_skipped_element_left_open_is_refused() {
        let mut skipped = Skipped::new(0);
        assert!(skipped.read(&Event::Eof, 0).is_err());
    }

    #[test]
    fn splice_tolerates_nested_cuts_and_inserts_ahead() {
        assert_eq!(
            splice(
                "abcdef",
                vec![(1..5, ""), (2..3, ""), (1..1, "X"), (5..6, "Y")]
            ),
            "aXY"
        );
    }

    #[test]
    fn a_checked_splice_refuses_overlaps_and_split_characters() {
        assert_eq!(
            splice_checked("abcdef", "p", vec![(4..5, "Y"), (0..1, "X")]).unwrap(),
            "XbcdYf"
        );
        assert!(splice_checked("abcdef", "p", vec![(0..3, ""), (2..4, "")]).is_err());
        assert!(splice_checked("é", "p", vec![(1..2, "")]).is_err());
        assert!(splice_checked("ab", "p", vec![(1..3, "")]).is_err());
    }
}
