//! [`UnsurfacedProperties`]: the edit that strips from a property part
//! everything [`fields`](super::fields) does not read, when metadata is
//! treated as content.

use std::ops::Range;

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use quick_xml::NsReader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::name::QName;

use super::walk::{Entered, Place, Walk};
use super::{FIRST_PID, resolve_ref};
use crate::opc::props::vocabulary::is_known;
use crate::opc::xml::edit::{
    AttributeExtent, Cut, Edit, Edited, Skipped, SpannedReader, attribute_extent, attribute_source,
    namespace_of, refuse_doctype,
};
use crate::opc::xml::ns::{DCTERMS, XSI, transitional};
use crate::opc::{attribute_value, malformed_xml};

/// The edit that strips from a property part everything no
/// [`fields`](super::fields) reads, applied when metadata is treated as
/// content, so what stays is exactly what detection sees.
///
/// It walks the part as [`fields`](super::fields) does, by local name in its
/// vocabulary, and keeps the root, each surfaced field with everything inside
/// it but what is in an unreviewed namespace, and the list containers,
/// vectors, and variants on the path to a kept list entry.
/// Every other element goes with its subtree, as do text, CDATA, and entity
/// references outside a field, an entity reference no field resolves, and
/// every processing instruction, and an array inside a field, whose bounds
/// nothing reads; whitespace between elements stays. A vector that loses
/// entries has its `size` set to the entries it keeps. A heading keeps its
/// count, written as the titles of parts it groups. A kept element keeps
/// only the attributes [`cut_attributes`] allows, none of them free text.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UnsurfacedProperties;

impl Edit for UnsurfacedProperties {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Properties)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        let mut reader = SpannedReader::new_ns(raw);
        let mut stripping = Stripping::new(raw);
        loop {
            let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;
            if !stripping.read(&reader.reader, namespace.as_deref(), &event, span)? {
                break;
            }
        }
        Ok(stripping.finish())
    }
}

/// An [`UnsurfacedProperties`] edit in progress over a part.
struct Stripping<'a> {
    /// The part's text.
    raw: &'a str,
    /// The open elements, which say what field an element is.
    walk: Walk,
    /// Each byte range cut, and what takes its place.
    cuts: Vec<Cut<Fill>>,
    /// The kept elements that are not fields: the root, and the list
    /// containers, vectors, and variants, kept only when an entry is.
    frames: Vec<Frame>,
    /// The vectors open inside the field being read, each keeping its
    /// `size` to the entries it holds.
    vectors: Vec<Frame>,
    /// The depth of the surfaced field being read, everything inside kept
    /// but an array.
    reading: Option<usize>,
    /// An element being removed, left out of the walk.
    removing: Option<Skipped>,
    /// How many custom properties are kept, which numbers each.
    properties: usize,
    /// The custom property being read.
    property: Option<OpenProperty>,
    /// The heading counts, written once the titles they group are known.
    counts: Vec<Count<'a>>,
    /// A heading count being read.
    counting: Option<Counting>,
    /// How many titles of parts are kept.
    titles: usize,
}

/// A custom property being read, which goes whole if its value is an array.
#[derive(Debug, Clone, Copy)]
struct OpenProperty {
    /// Where its start tag begins.
    start: usize,
    /// How many cuts there were before it, which dropping it truncates to.
    cuts: usize,
}

impl<'a> Stripping<'a> {
    /// An edit of `raw`, nothing read yet.
    fn new(raw: &'a str) -> Self {
        Self {
            raw,
            walk: Walk::default(),
            cuts: Vec::new(),
            frames: Vec::new(),
            vectors: Vec::new(),
            reading: None,
            removing: None,
            properties: 0,
            property: None,
            counts: Vec::new(),
            counting: None,
            titles: 0,
        }
    }

    /// Read `event`, spanning `span`, its name bound to `namespace` as
    /// `reader` resolves it: `false` once the part ends.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the part is not well-formed XML or declares a document type.
    fn read(
        &mut self,
        reader: &NsReader<&[u8]>,
        namespace: Option<&str>,
        event: &Event<'_>,
        span: Range<usize>,
    ) -> Result<bool> {
        // Before every branch: a heading count's content is rewritten, not
        // read, so a declaration inside it must not pass unseen.
        refuse_doctype(event)?;
        if let Some(counting) = &mut self.counting {
            if counting.read(event)?
                && let Some(counting) = self.counting.take()
            {
                self.counts.push(counting.into_count(span.start));
                self.walk.leave();
            }
            return Ok(true);
        }
        if Skipped::skip(&mut self.removing, event, span.end, &mut self.cuts)? {
            return Ok(true);
        }
        match event {
            Event::Eof => return Ok(false),
            Event::Start(e) => self.on_start(reader, namespace, e, span, true)?,
            Event::Empty(e) => self.on_start(reader, namespace, e, span, false)?,
            Event::End(_) => self.on_end(span),
            Event::Text(text) if self.reading.is_none() && !is_blank(text) => self.remove(span),
            Event::CData(_) if self.reading.is_none() => self.remove(span),
            Event::GeneralRef(r) if self.reading.is_none() || resolve_ref(r).is_empty() => {
                self.remove(span);
            }
            Event::PI(_) => self.remove(span),
            _ => {}
        }
        Ok(true)
    }

    /// Cut `span` out of the part.
    fn remove(&mut self, span: Range<usize>) {
        self.cuts.push(Cut::remove(span));
    }

    /// Read the start tag `e` spanning `span`, of an element that holds
    /// content (`is_start`) or an empty one.
    fn on_start(
        &mut self,
        reader: &NsReader<&[u8]>,
        namespace: Option<&str>,
        e: &BytesStart<'_>,
        span: Range<usize>,
        is_start: bool,
    ) -> Result<()> {
        let depth = self.walk.depth();
        let field = self.walk.enter(namespace, e);
        let local = e.local_name().into_inner();
        let namespace = namespace.map(transitional);
        // A heading's count is kept, its value written from the titles it
        // groups.
        if self.reading.is_none() && self.walk.is_count() {
            keep_child(&mut self.frames);
            let element = Element {
                namespace,
                local,
                role: Role::Other,
            };
            cut_attributes(reader, self.raw, e, &element, &mut self.cuts)?;
            self.on_count(e, span, is_start);
            return Ok(());
        }
        // An array is a custom property's whole value, and nothing reads its
        // bounds: the property goes with it, as one with no value does.
        if local == "array"
            && self.reading.is_some_and(|reading| reading + 1 == depth)
            && let Some(OpenProperty { start, cuts }) = self.property.take()
        {
            self.walk.leave();
            self.walk.leave();
            self.cuts.truncate(cuts);
            self.properties -= 1;
            self.reading = None;
            self.removing = Some(Skipped::open(start, if is_start { 2 } else { 1 }));
            return Ok(());
        }
        let Some(role) = self.role(depth, field, local, is_start) else {
            self.walk.leave();
            self.removing = Skipped::begin(span, is_start, &mut self.cuts);
            return Ok(());
        };
        if let Some(vector) = self.vectors.last_mut()
            && vector.depth + 1 == depth
        {
            vector.children += 1;
        }
        let sized = local == "vector";
        if !is_start {
            self.walk.leave();
        } else if self.reading.is_some_and(|field| field < depth) {
            if sized {
                let size = size(self.raw, e)?;
                self.vectors
                    .push(Frame::new(depth, span.start, self.cuts.len(), size));
            }
        } else if self.reading.is_none() {
            let size = if sized { size(self.raw, e)? } else { None };
            self.frames
                .push(Frame::new(depth, span.start, self.cuts.len(), size));
        }
        if matches!(role, Role::Property(_)) && is_start {
            self.property = Some(OpenProperty {
                start: span.start,
                cuts: self.cuts.len(),
            });
        }
        let role = if sized && is_start {
            Role::Vector
        } else {
            role
        };
        let element = Element {
            namespace,
            local,
            role,
        };
        cut_attributes(reader, self.raw, e, &element, &mut self.cuts)
    }

    /// Begin reading the heading count `e` spanning `span`: a started one is
    /// read to its end tag, an empty one is a count of none, closed once it
    /// holds a value.
    fn on_count(&mut self, e: &BytesStart<'_>, span: Range<usize>, is_start: bool) {
        if is_start {
            self.counting = Some(Counting::new(span.end));
        } else {
            self.walk.leave();
            self.counts.push(Count {
                range: span.end - 2..span.end,
                close: Some(&self.raw[span.start + 1..][..e.name().as_ref().len()]),
                said: None,
            });
        }
    }

    /// How the element `local`, opening at `depth` and holding `field`, is
    /// kept, or `None` when it goes.
    fn role(
        &mut self,
        depth: usize,
        field: Option<Entered>,
        local: &str,
        is_start: bool,
    ) -> Option<Role> {
        if self.reading.is_some() {
            // An array's bounds are numbers nothing reads, and nothing reads
            // an unreviewed vocabulary.
            return (local != "array" && !self.walk.is_foreign()).then_some(Role::Other);
        }
        if depth == 0 {
            return Some(Role::Other);
        }
        let Some(field) = field else {
            return (is_start && self.walk.is_list_path()).then_some(Role::Other);
        };
        // A custom property's name is read only when it holds a value, so an
        // empty one is never surfaced.
        if !is_start && field.place == Place::Custom {
            return None;
        }
        keep_child(&mut self.frames);
        if is_start {
            self.reading = Some(depth);
        }
        match field.place {
            Place::Custom => {
                let pid = FIRST_PID + self.properties;
                self.properties += 1;
                Some(Role::Property(pid))
            }
            Place::Entry => {
                if self.walk.list() == Some("TitlesOfParts") {
                    self.titles += 1;
                }
                Some(Role::Other)
            }
            Place::Element => Some(Role::Other),
        }
    }

    /// Read an end tag spanning `span`.
    fn on_end(&mut self, span: Range<usize>) {
        self.walk.leave();
        let depth = self.walk.depth();
        if self.reading == Some(depth) {
            self.reading = None;
            self.property = None;
        } else if self.reading.is_some() {
            if let Some(vector) = self.vectors.pop_if(|vector| vector.depth == depth) {
                vector.resize(&mut self.cuts);
            }
        } else if let Some(frame) = self.frames.pop_if(|frame| frame.depth == depth) {
            if frame.children == 0 && depth > 0 {
                self.cuts.truncate(frame.cuts);
                self.cuts.push(Cut::remove(frame.start..span.end));
            } else {
                frame.resize(&mut self.cuts);
                keep_child(&mut self.frames);
            }
        }
    }

    /// The part as edited, the heading counts written, or `None` when the
    /// edit changes nothing.
    fn finish(mut self) -> Option<Edited> {
        write_counts(self.raw, &self.counts, self.titles, &mut self.cuts);
        let count = self.cuts.iter().filter(|cut| cut.with.counted()).count();
        Edited::from_cuts(self.raw, self.cuts, count)
    }
}

/// A heading's count being read: where its content begins, how many elements
/// are open inside it, itself included, and its character data as XML reads
/// it (none once an element or a reference that does not decode is met).
struct Counting {
    start: usize,
    depth: usize,
    text: Option<String>,
}

impl Counting {
    /// A count whose content begins at `start`.
    fn new(start: usize) -> Self {
        Self {
            start,
            depth: 1,
            text: Some(String::new()),
        }
    }

    /// Read `event` inside the count: `true` once it closes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the part ends inside it.
    fn read(&mut self, event: &Event<'_>) -> Result<bool> {
        match event {
            Event::Start(_) => {
                self.depth += 1;
                self.text = None;
            }
            Event::Empty(_) => self.text = None,
            Event::End(_) => self.depth -= 1,
            Event::Eof => return Err(malformed_xml("unclosed element")),
            Event::Text(t) => {
                let decoded = unescape(t.as_ref()).ok();
                self.text = self.text.take().zip(decoded).map(|(text, t)| text + &t);
            }
            Event::CData(t) => {
                if let Some(text) = &mut self.text {
                    text.push_str(t.as_ref());
                }
            }
            Event::GeneralRef(r) => {
                let resolved = resolve_ref(r);
                if resolved.is_empty() {
                    self.text = None;
                } else if let Some(text) = &mut self.text {
                    text.push_str(&resolved);
                }
            }
            _ => {}
        }
        Ok(self.depth == 0)
    }

    /// The count read, its content ending at `end`.
    fn into_count<'a>(self, end: usize) -> Count<'a> {
        Count {
            range: self.start..end,
            close: None,
            said: self
                .text
                .as_deref()
                .and_then(|text| text.trim().parse().ok()),
        }
    }
}

/// A heading's count in a property part.
struct Count<'a> {
    /// What holds its value: its content, or the `/>` of an empty one.
    range: Range<usize>,
    /// For an empty one, its name, to close it with once it holds a value.
    close: Option<&'a str>,
    /// The number its character data says, if it says one.
    said: Option<usize>,
}

/// Write each of `counts` as the titles of parts its heading groups: of the
/// `titles` kept, in order, as many as it says, the last heading taking the
/// rest. A count that is no number groups none, so none carries data.
fn write_counts(raw: &str, counts: &[Count<'_>], titles: usize, cuts: &mut Vec<Cut<Fill>>) {
    let mut rest = titles;
    for (at, count) in counts.iter().enumerate() {
        let written = &raw[count.range.clone()];
        let value = if at + 1 == counts.len() {
            rest
        } else {
            count.said.unwrap_or(0).min(rest)
        };
        rest -= value;
        let value = value.to_string();
        match count.close {
            None if written == value => {}
            None => cuts.push(Cut::new(count.range.clone(), Fill::Replaced(value))),
            Some(name) => cuts.push(Cut::new(
                count.range.clone(),
                Fill::Replaced(format!(">{value}</{name}>")),
            )),
        }
    }
}

/// A kept element of a property part, as its attributes are decided.
struct Element<'a> {
    /// Its namespace, a strict one as its transitional twin.
    namespace: Option<&'a str>,
    local: &'a str,
    /// What its attributes are kept for.
    role: Role,
}

/// What a kept element of a property part is, for its attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// A vector whose `size` is kept to its entries.
    Vector,
    /// A kept custom property, and the pid it is given.
    Property(usize),
    /// Any other element.
    Other,
}

/// The format id every custom property a person adds carries
/// (`FMTID_UserDefinedProperties`).
const USER_DEFINED: &str = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}";

/// The variant types a vector or an array may be of.
const BASE_TYPES: &[&str] = &[
    "variant", "i1", "i2", "i4", "i8", "int", "ui1", "ui2", "ui4", "ui8", "uint", "r4", "r8",
    "decimal", "lpstr", "lpwstr", "bstr", "date", "filetime", "bool", "cy", "error", "clsid",
];

/// What a property part does with an attribute of a kept element.
enum AttributeEdit {
    /// Keep the attribute as it is.
    Keep,
    /// Give the attribute this value, in place of one that could carry text.
    Replace(String),
    /// Remove the attribute.
    Remove,
}

/// Cut from the kept `element` (its start tag `e`) each attribute no reader
/// reads and its structure does not need, and give a structural number or
/// id its own value, so no attribute value carries text:
///
/// - a namespace declaration of a reviewed vocabulary, or an empty one;
/// - `xsi:type` on a DC terms element, only as `W3CDTF` in DC terms;
/// - a custom property's `name` (read as a field), its `pid` (renumbered by
///   position), and its `fmtid` (the user-defined format id);
/// - a vector's `size` (kept to its entries) and its `baseType` when it is a
///   variant type.
fn cut_attributes(
    reader: &NsReader<&[u8]>,
    raw: &str,
    e: &BytesStart<'_>,
    element: &Element<'_>,
    cuts: &mut Vec<Cut<Fill>>,
) -> Result<()> {
    for attr in e.attributes() {
        let attr = attr.map_err(malformed_xml)?;
        let AttributeExtent {
            whole: extent,
            value: value_range,
        } = attribute_extent(raw, &attr)?;
        let written = &raw[value_range.clone()];
        let value = attribute_value(written).ok();
        let value = value.as_deref();
        let edit = if attr.key.as_namespace_binding().is_some() {
            let reviewed = value.is_some_and(|uri| uri.is_empty() || is_known(transitional(uri)));
            if reviewed {
                AttributeEdit::Keep
            } else {
                AttributeEdit::Remove
            }
        } else {
            let (resolved, local) = reader.resolver().resolve_attribute(attr.key);
            let qualified = namespace_of(&resolved);
            match (element.local, qualified.as_deref(), local.into_inner()) {
                (_, Some(XSI), "type")
                    if element.namespace == Some(DCTERMS) && is_w3cdtf(reader, value) =>
                {
                    AttributeEdit::Keep
                }
                (_, None, "name") if matches!(element.role, Role::Property(_)) => {
                    AttributeEdit::Keep
                }
                (_, None, "pid") => match element.role {
                    Role::Property(pid) if written == pid.to_string() => AttributeEdit::Keep,
                    Role::Property(pid) => AttributeEdit::Replace(pid.to_string()),
                    _ => AttributeEdit::Remove,
                },
                (_, None, "fmtid") if matches!(element.role, Role::Property(_)) => {
                    if written == USER_DEFINED {
                        AttributeEdit::Keep
                    } else {
                        AttributeEdit::Replace(USER_DEFINED.to_owned())
                    }
                }
                // Kept to the vector's entries when it closes.
                (_, None, "size") if element.role == Role::Vector => AttributeEdit::Keep,
                ("vector", None, "baseType") if value.is_some_and(|t| BASE_TYPES.contains(&t)) => {
                    AttributeEdit::Keep
                }
                _ => AttributeEdit::Remove,
            }
        };
        match edit {
            AttributeEdit::Keep => {}
            AttributeEdit::Replace(value) => {
                cuts.push(Cut::new(value_range, Fill::Replaced(value)))
            }
            AttributeEdit::Remove => cuts.push(Cut::remove(extent)),
        }
    }
    Ok(())
}

/// Whether `value`, an `xsi:type`, names DC terms' `W3CDTF` by a prefix bound
/// where it is written.
fn is_w3cdtf(reader: &NsReader<&[u8]>, value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let (local, prefix) = QName(value).decompose();
    if local.into_inner() != "W3CDTF" {
        return false;
    }
    // An empty prefix (`:W3CDTF`) is no prefix, resolved as the default.
    let prefix = prefix.filter(|prefix| !prefix.as_ref().is_empty());
    let resolved = reader.resolver().resolve_prefix(prefix, true);
    namespace_of(&resolved).as_deref() == Some(DCTERMS)
}

/// What an [`UnsurfacedProperties`] edit writes in place of a range: nothing
/// for what it removes, a value in place of one that could carry text (both
/// counted), or a vector's new `size` (not counted).
#[derive(Default)]
enum Fill {
    #[default]
    Removed,
    Replaced(String),
    Written(String),
}

impl Fill {
    /// Whether this cut removes content.
    fn counted(&self) -> bool {
        !matches!(self, Self::Written(_))
    }
}

impl AsRef<str> for Fill {
    fn as_ref(&self) -> &str {
        match self {
            Self::Removed => "",
            Self::Replaced(text) | Self::Written(text) => text,
        }
    }
}

/// An open element an [`UnsurfacedProperties`] edit keeps only if it keeps
/// something inside it: the root (kept regardless), or a list container,
/// vector, or variant.
struct Frame {
    /// How many elements are open around it.
    depth: usize,
    /// Where its start tag begins.
    start: usize,
    /// How many cuts were made before it opened.
    cuts: usize,
    /// For a vector, its `size` as written.
    size: Option<Size>,
    /// How many of its children are kept.
    children: usize,
}

impl Frame {
    fn new(depth: usize, start: usize, cuts: usize, size: Option<Size>) -> Self {
        Self {
            depth,
            start,
            cuts,
            size,
            children: 0,
        }
    }

    /// Set the `size` this vector was written with to the children it keeps.
    fn resize(self, cuts: &mut Vec<Cut<Fill>>) {
        if let Some(size) = self.size
            && size.written != self.children.to_string()
        {
            cuts.push(Cut::new(
                size.range,
                Fill::Written(self.children.to_string()),
            ));
        }
    }
}

/// A vector's `size` attribute as written: its value's range and text.
struct Size {
    range: Range<usize>,
    written: String,
}

/// Count a kept child of the innermost open frame.
fn keep_child(frames: &mut [Frame]) {
    if let Some(frame) = frames.last_mut() {
        frame.children += 1;
    }
}

/// `e`'s unqualified `size` attribute, if it has one.
fn size(raw: &str, e: &BytesStart<'_>) -> Result<Option<Size>> {
    for attribute in e.attributes() {
        let attribute = attribute.map_err(malformed_xml)?;
        if attribute.key.as_ref() == "size" {
            let range = attribute_source(raw, &attribute)?.value;
            let written = raw[range.clone()].to_owned();
            return Ok(Some(Size { range, written }));
        }
    }
    Ok(None)
}

/// Whether `text` is only whitespace.
fn is_blank(text: &BytesText<'_>) -> bool {
    text.as_ref().chars().all(|c| c.is_ascii_whitespace())
}
