//! Reading, labelling, and clearing an OOXML document-property part
//! (`docProps/core.xml`, `docProps/app.xml`, `docProps/custom.xml`).
//!
//! These parts hold named fields that carry personal data: the author and last
//! editor, the timestamps, the company and manager, free text (the title,
//! description, keywords, template path), the names of the document's parts,
//! and custom properties. This says what each field is, reads the fields out of
//! a part's bytes, and edits them back, matching each element by its local
//! name in its own vocabulary (`dc:`, `dcterms:`, `cp:`, the extended or
//! custom properties, `vt:`) under whatever prefix binds it, and tolerating a
//! leading byte-order mark. An element in an unreviewed namespace holds
//! nothing read. When metadata
//! is treated as content, [`UnsurfacedProperties`] strips everything else by
//! the same walk, so the part keeps exactly what [`fields`] reads.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::{io, mem};

use bytes::Bytes;
use elide_core::Result;
use elide_core::entity::{LabelRef, builtins};
use elide_core::redaction::HiddenContent;
use quick_xml::NsReader;
use quick_xml::escape::{resolve_predefined_entity, unescape};
use quick_xml::events::{BytesRef, BytesStart, BytesText, Event};
use quick_xml::name::QName;
use quick_xml::writer::Writer;

use crate::opc::ns::{
    CORE_PROPERTIES, CUSTOM_PROPERTIES, DC, DCTERMS, DOC_PROPS_VTYPES, EXTENDED_PROPERTIES, XSI,
    transitional,
};
use crate::opc::vocabulary::is_known;
use crate::opc::xml_edit::{
    AttributeExtent, Cut, Edit, Edited, Skipped, SpannedReader, attribute_extent, attribute_source,
    bom_len, bound, local_name, namespace_of, refuse_doctype,
};
use crate::opc::{attribute_value, malformed_xml};

/// The core and extended property elements surfaced as fields, each holding
/// its value as text.
const ELEMENT_FIELDS: &[&str] = &[
    "creator",
    "lastModifiedBy",
    "Manager",
    "created",
    "modified",
    "Company",
    "title",
    "subject",
    "description",
    "keywords",
    "category",
    "Template",
    "HyperlinkBase",
];

/// The element fields that hold a typed W3CDTF timestamp, which is cleared
/// rather than overwritten with arbitrary text.
const TIMESTAMPS: &[&str] = &["created", "modified"];

/// The variant types (`vt:`) that hold free text: a list entry, or a custom
/// property that can be overwritten. A custom property of any other type (a
/// number, a date, a flag, a list) is removed instead.
const TEXT_TYPES: &[&str] = &["lpwstr", "lpstr", "bstr"];

/// The extended properties that list the document's parts: `TitlesOfParts`
/// names each part (a sheet, a slide title), and `HeadingPairs` says how those
/// names group (`Worksheets`, 3). Kept together, since each describes the other.
const LISTS: &[&str] = &["TitlesOfParts", "HeadingPairs"];

/// A custom property's element, named by its `name` attribute.
const CUSTOM_PROPERTY: &str = "property";

/// The pid of the first custom property: the pids below it are reserved.
const FIRST_PID: usize = 2;

/// Where a field's value sits in its part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// A core or extended property element, its value its text.
    Element,
    /// A text entry of a list, keyed `<list>/<index>` (`TitlesOfParts/2`).
    Entry,
    /// A custom property, keyed by its position, its value in one typed
    /// child.
    Custom,
}

impl Place {
    /// What a field in this place, held by the element `local`, is: an
    /// element field by the schema's meaning for it, any other field free
    /// content.
    fn label(self, local: &str) -> LabelRef {
        let label = match (self, local) {
            (Self::Element, "creator" | "lastModifiedBy" | "Manager") => &builtins::PERSON_NAME,
            (Self::Element, "created" | "modified") => &builtins::DATE_TIME,
            (Self::Element, "Company") => &builtins::ORGANIZATION_NAME,
            _ => &builtins::COMMUNICATIONS_CONTENT,
        };
        LabelRef::from(&**label)
    }
}

/// A property field: its key, its value, and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    /// The field's key: an element's local name, `<list>/<n>`, or a custom
    /// property's `property/<n>` (its name `property/<n>/name`).
    pub(crate) key: String,
    /// The field's value, trimmed.
    pub(crate) value: String,
    /// What the field is.
    pub(crate) label: LabelRef,
}

/// The edit that strips from a property part everything no [`fields`] reads,
/// applied when metadata is treated as content, so what stays is exactly what
/// detection sees.
///
/// It walks the part as [`fields`] does, by local name in its vocabulary, and
/// keeps the root, each surfaced field with everything inside it but what
/// is in an unreviewed namespace, and the
/// list containers, vectors, and variants on the path to a kept list entry.
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
    /// The heading counts, written once the titles they group are known.
    counts: Vec<Count<'a>>,
    /// A heading count being read.
    counting: Option<Counting>,
    /// How many titles of parts are kept.
    titles: usize,
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
        if let Some(counting) = &mut self.counting {
            if counting.read(event)?
                && let Some(counting) = self.counting.take()
            {
                self.counts.push(counting.into_count(span.start));
                self.walk.leave();
            }
            return Ok(true);
        }
        refuse_doctype(event)?;
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

/// A field an element holds: its key in the part, where its value sits, what
/// it is, whether it holds a typed timestamp, and for a custom property, its
/// name.
struct Entered {
    key: String,
    place: Place,
    label: LabelRef,
    timestamp: bool,
    name: Option<String>,
}

impl Entered {
    /// The field `key` in `place`, held by the element `local`.
    fn new(key: String, place: Place, local: &str) -> Self {
        Self {
            key,
            place,
            label: place.label(local),
            timestamp: place == Place::Element && TIMESTAMPS.contains(&local),
            name: None,
        }
    }

    /// The key of the field holding a custom property's name.
    fn name_key(&self) -> String {
        format!("{}/name", self.key)
    }
}

/// An open element of a property part, as a [`Walk`] matches it.
#[derive(Debug)]
enum Open {
    /// An element in an unreviewed namespace, or inside one.
    Foreign,
    /// An element out of the vocabulary its local name belongs to, which
    /// matches no name.
    Misplaced,
    /// An element in its vocabulary, by its local name.
    Known(String),
}

impl Open {
    /// Whether this is the element `local`.
    fn is(&self, local: &str) -> bool {
        matches!(self, Self::Known(known) if known == local)
    }

    /// Whether this is a list of the document's parts.
    fn is_list(&self) -> bool {
        self.list().is_some()
    }

    /// The list of the document's parts this is, if it is one.
    fn list(&self) -> Option<&str> {
        match self {
            Self::Known(local) if LISTS.contains(&local.as_str()) => Some(local),
            _ => None,
        }
    }
}

/// The open elements of a property part, which say what field an element is.
#[derive(Debug, Default)]
struct Walk {
    /// The open elements, the root first.
    open: Vec<Open>,
    /// How many text entries of the current list have been entered.
    entries: usize,
    /// How many named custom properties have been entered.
    properties: usize,
    /// How many times each element field has been entered.
    seen: HashMap<String, usize>,
    /// Where the outermost open element in an unreviewed namespace sits:
    /// nothing in it is a field or read.
    outside: Option<usize>,
}

/// The namespace an element this walk reads by name must be in to be that
/// element, a strict one read as its transitional twin.
fn vocabulary(local: &str) -> Option<&'static str> {
    match local {
        "creator" | "title" | "subject" | "description" => Some(DC),
        "lastModifiedBy" | "keywords" | "category" => Some(CORE_PROPERTIES),
        "created" | "modified" => Some(DCTERMS),
        "Manager" | "Company" | "Template" | "HyperlinkBase" | "TitlesOfParts" | "HeadingPairs" => {
            Some(EXTENDED_PROPERTIES)
        }
        CUSTOM_PROPERTY => Some(CUSTOM_PROPERTIES),
        "vector" | "variant" | "array" | "lpwstr" | "lpstr" | "bstr" | "i4" => {
            Some(DOC_PROPS_VTYPES)
        }
        _ => None,
    }
}

impl Walk {
    /// Enter `elem`, in `namespace`, returning the field it holds, if any.
    ///
    /// An element is the one its local name says only in that name's
    /// vocabulary; in another namespace it is no field, and one in an
    /// unreviewed namespace, or in none, holds nothing read.
    ///
    /// Every key is unique in its part. An element field is keyed by its name,
    /// a repeated one with `#<n>` after it (no element name holds a `#`). A
    /// list entry is keyed `<list>/<n>`, and a custom property by its position,
    /// `property/<n>`, since its name is free text that could look like any
    /// key; the name is a field of its own, `property/<n>/name`.
    fn enter(&mut self, namespace: Option<&str>, elem: &BytesStart<'_>) -> Option<Entered> {
        let namespace = namespace.map(transitional);
        if self.outside.is_some() || namespace.is_none_or(|namespace| !is_known(namespace)) {
            self.outside.get_or_insert(self.open.len());
            self.open.push(Open::Foreign);
            return None;
        }
        let local = local_name(elem);
        let (entered, open) = match vocabulary(&local) {
            Some(expected) if namespace != Some(expected) => (None, Open::Misplaced),
            _ => (self.field(&local, elem), Open::Known(local)),
        };
        self.open.push(open);
        entered
    }

    /// The field the element `local` (`elem`), opening in its vocabulary,
    /// holds, if any.
    fn field(&mut self, local: &str, elem: &BytesStart<'_>) -> Option<Entered> {
        if self.open.len() == 1 {
            return self.property(local, elem);
        }
        // A list's entries sit in its vector, a heading's in a variant.
        let in_list = matches!(self.open.len(), 3 | 4) && self.is_list_path();
        if !in_list || !TEXT_TYPES.contains(&local) {
            return None;
        }
        let key = format!("{}/{}", self.list()?, self.entries);
        self.entries += 1;
        Some(Entered::new(key, Place::Entry, local))
    }

    /// The field the element `local` (`elem`), opening as a child of the
    /// root, holds, if any: an element field or a named custom property. A
    /// list starts its entries.
    fn property(&mut self, local: &str, elem: &BytesStart<'_>) -> Option<Entered> {
        if ELEMENT_FIELDS.contains(&local) {
            let count = self.seen.entry(local.to_owned()).or_default();
            *count += 1;
            let key = match *count {
                1 => local.to_owned(),
                n => format!("{local}#{n}"),
            };
            return Some(Entered::new(key, Place::Element, local));
        }
        if local == CUSTOM_PROPERTY {
            let name = elem
                .try_get_attribute("name")
                .ok()
                .flatten()
                .and_then(|name| attribute_value(&name.value).ok().map(Cow::into_owned))?;
            let key = format!("{CUSTOM_PROPERTY}/{}", self.properties);
            self.properties += 1;
            return Some(Entered {
                name: Some(name),
                ..Entered::new(key, Place::Custom, local)
            });
        }
        if LISTS.contains(&local) {
            self.entries = 0;
        }
        None
    }

    /// Leave the innermost open element.
    fn leave(&mut self) {
        self.open.pop();
        if self.outside == Some(self.open.len()) {
            self.outside = None;
        }
    }

    /// Whether an open element is in an unreviewed namespace.
    fn is_foreign(&self) -> bool {
        self.outside.is_some()
    }

    /// How many elements are open.
    fn depth(&self) -> usize {
        self.open.len()
    }

    /// The list of the document's parts an open element is, if any.
    fn list(&self) -> Option<&str> {
        match self.open.as_slice() {
            [_, list, ..] => list.list(),
            _ => None,
        }
    }

    /// Whether the innermost open element is a custom property.
    fn is_property(&self) -> bool {
        matches!(self.open.as_slice(), [_, property] if property.is(CUSTOM_PROPERTY))
    }

    /// Whether the innermost open element is a heading's count: an `i4`
    /// variant of the heading pairs' vector.
    fn is_count(&self) -> bool {
        matches!(
            self.open.as_slice(),
            [_, pairs, vector, variant, count]
                if pairs.is("HeadingPairs")
                    && vector.is("vector")
                    && variant.is("variant")
                    && count.is("i4")
        )
    }

    /// Whether the innermost open element is a list container, its vector,
    /// or a variant of that vector: the path to a list entry.
    fn is_list_path(&self) -> bool {
        match self.open.as_slice() {
            [_, list] => list.is_list(),
            [_, list, vector] => list.is_list() && vector.is("vector"),
            [_, list, vector, variant] => {
                list.is_list() && vector.is("vector") && variant.is("variant")
            }
            _ => false,
        }
    }
}

/// A field being read: where it opened and what has been read of it.
struct Reading {
    field: Entered,
    depth: usize,
    value: String,
}

/// The fields a property part surfaces, in document order. Only fields with a
/// non-empty value are returned, so an already-empty `<dc:creator/>` is not
/// surfaced as a redaction subject.
#[must_use]
pub(crate) fn fields(xml: &[u8]) -> Vec<Field> {
    // Not valid UTF-8: no fields are readable (a property part is always UTF-8).
    let Some((_, text)) = split_bom(xml) else {
        return Vec::new();
    };
    let mut reader = NsReader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut fields = Vec::new();
    let mut walk = Walk::default();
    let mut reading: Option<Reading> = None;

    loop {
        let read = reader
            .read_resolved_event()
            .map(|(resolved, event)| (bound(&resolved), event));
        match read {
            Ok((namespace, Event::Start(elem))) => {
                let field = walk.enter(namespace.as_deref(), &elem);
                // A custom property holding a list reads one entry per line,
                // so no match runs from one entry into the next.
                if let Some(open) = &mut reading
                    && is_text_type(&elem)
                    && !open.value.is_empty()
                {
                    open.value.push('\n');
                }
                if reading.is_none() {
                    if let Some(field) = &field
                        && let Some(name) = field.name.as_deref().map(str::trim)
                        && !name.is_empty()
                    {
                        fields.push(Field {
                            key: field.name_key(),
                            value: name.to_owned(),
                            label: field.label.clone(),
                        });
                    }
                    reading = field.map(|field| Reading {
                        field,
                        depth: walk.depth(),
                        value: String::new(),
                    });
                }
            }
            Ok((namespace, Event::Empty(elem))) => {
                walk.enter(namespace.as_deref(), &elem);
                walk.leave();
            }
            Ok((_, Event::Text(t))) => {
                if let Some(open) = &mut reading
                    && !walk.is_foreign()
                {
                    open.value
                        .push_str(&unescape(t.as_ref()).unwrap_or_default());
                }
            }
            // A field value written as CDATA carries its text verbatim (no entity
            // escaping), so it is appended as-is; without this a `<dc:creator>`
            // stored as CDATA would read as empty and never be surfaced.
            Ok((_, Event::CData(t))) => {
                if let Some(open) = &mut reading
                    && !walk.is_foreign()
                {
                    open.value.push_str(&t);
                }
            }
            // quick-xml emits an entity reference (`&#233;`, `&amp;`) as its own
            // event, not as part of the surrounding text. Resolve it into the
            // value; without this a name like `Jos&#233;`, or a wholly
            // entity-encoded value, would be read short and slip past detection.
            Ok((_, Event::GeneralRef(r))) => {
                if let Some(open) = &mut reading
                    && !walk.is_foreign()
                {
                    open.value.push_str(&resolve_ref(&r));
                }
            }
            Ok((_, Event::End(_))) => {
                if reading
                    .as_ref()
                    .is_some_and(|open| walk.depth() == open.depth)
                    && let Some(open) = reading.take()
                    && !open.value.trim().is_empty()
                {
                    fields.push(Field {
                        key: open.field.key,
                        value: open.value.trim().to_owned(),
                        label: open.field.label,
                    });
                }
                walk.leave();
            }
            Ok((_, Event::Eof)) | Err(_) => break,
            _ => {}
        }
    }
    fields
}

/// Write each field's new value (by key), a blank value removing the field,
/// returning the edited XML.
///
/// An element field or a list entry keeps its element: its new
/// value becomes its only text, and a blank one, or any value for a field
/// holding a typed timestamp, leaves it empty. A custom property takes its new
/// value in its typed value when that holds text, and is otherwise dropped
/// whole, since an empty or mistyped value is not valid, and those after it
/// are numbered past every pid before it, so none repeats and the numbering
/// has no gap. A key that names no field is a no-op; the rest of the
/// document is preserved in structure.
///
/// Fails closed: on non-UTF-8 input or a mid-stream parse error, the original
/// bytes are returned unchanged rather than a blank or truncated part, since a
/// metadata part that cannot be rewritten is better left intact than corrupted.
#[must_use]
pub(crate) fn rewrite(xml: &[u8], fields: &[(&str, &str)]) -> Bytes {
    // Not valid UTF-8: fail closed by preserving the original bytes rather than
    // emitting a blank part. A property part is always UTF-8 XML in practice.
    let Some((bom, text)) = split_bom(xml) else {
        return Bytes::copy_from_slice(xml);
    };
    match rewritten(bom, text, fields) {
        Some(rewritten) => Bytes::from(rewritten),
        None => Bytes::copy_from_slice(xml),
    }
}

/// The part `text`, after its byte-order mark `bom`, with `fields` written as
/// [`rewrite`] writes them, or `None` when it does not read or write.
fn rewritten(bom: &str, text: &str, fields: &[(&str, &str)]) -> Option<Vec<u8>> {
    let mut reader = NsReader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut rewriting = Rewriting {
        fields,
        // The output keeps the part's BOM, written ahead of everything else.
        writer: Writer::new(bom.as_bytes().to_vec()),
        walk: Walk::default(),
        mode: Mode::Copy,
        pids: Pids::default(),
    };
    loop {
        let (resolved, event) = reader.read_resolved_event().ok()?;
        if matches!(event, Event::Eof) {
            break;
        }
        let namespace = bound(&resolved);
        rewriting.event(namespace.as_deref(), event).ok()?;
    }
    Some(rewriting.writer.into_inner())
}

/// A [`rewrite`] in progress over a part.
struct Rewriting<'v> {
    /// The new value of each field, by key.
    fields: &'v [(&'v str, &'v str)],
    writer: Writer<Vec<u8>>,
    walk: Walk,
    mode: Mode<'v>,
    /// The pids of the custom properties written.
    pids: Pids,
}

/// What a [`rewrite`] does with the events it reads.
enum Mode<'v> {
    /// Write each as it is.
    Copy,
    /// Clear a field, this many elements open in it, itself included: its
    /// text is dropped and its elements are written empty.
    Clearing(usize),
    /// Drop a custom property, this many elements open in it, itself
    /// included: nothing of it is written.
    Dropping(usize),
    /// Hold back a custom property being given this value until its typed
    /// child says whether the value fits.
    Held(BytesStart<'static>, &'v str),
}

impl<'v> Rewriting<'v> {
    /// The new value of the field `key`, trimmed, if it is given one.
    fn value_of(&self, key: &str) -> Option<&'v str> {
        self.fields
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.trim())
    }

    /// Write `event`.
    fn write(&mut self, event: Event<'_>) -> io::Result<()> {
        self.writer.write_event(event)
    }

    /// Rewrite `event`, its name bound to `namespace`.
    fn event(&mut self, namespace: Option<&str>, event: Event<'_>) -> io::Result<()> {
        match mem::replace(&mut self.mode, Mode::Copy) {
            Mode::Dropping(depth) => {
                self.dropping(depth, namespace, &event);
                Ok(())
            }
            Mode::Held(property, value) => self.held(property, value, namespace, event),
            mode @ (Mode::Copy | Mode::Clearing(_)) => {
                self.mode = mode;
                self.copy(namespace, event)
            }
        }
    }

    /// Read `event` inside a custom property being dropped, `depth` elements
    /// open in it.
    fn dropping(&mut self, depth: usize, namespace: Option<&str>, event: &Event<'_>) {
        let depth = match event {
            Event::Start(elem) => {
                self.walk.enter(namespace, elem);
                depth + 1
            }
            Event::End(_) => {
                self.walk.leave();
                depth - 1
            }
            _ => depth,
        };
        if depth > 0 {
            self.mode = Mode::Dropping(depth);
        }
    }

    /// Drop the custom property open, `depth` elements of it open.
    fn drop_property(&mut self, depth: usize) {
        self.mode = Mode::Dropping(depth);
        self.pids.dropped = true;
    }

    /// Read `event` after the start of the custom property `property`, held
    /// back to take `value`: a typed child holding text takes it, any other
    /// drops the property.
    fn held(
        &mut self,
        property: BytesStart<'static>,
        value: &'v str,
        namespace: Option<&str>,
        event: Event<'_>,
    ) -> io::Result<()> {
        match event {
            Event::Start(elem) if is_text_type(&elem) => {
                self.walk.enter(namespace, &elem);
                let property = self.pids.number(property);
                self.write(Event::Start(property))?;
                self.write(Event::Start(elem))?;
                self.write(Event::Text(BytesText::new(value)))?;
                self.mode = Mode::Clearing(2);
            }
            Event::Start(elem) => {
                self.walk.enter(namespace, &elem);
                self.drop_property(2);
            }
            Event::Empty(_) => self.drop_property(1),
            // A property with no typed value goes.
            Event::End(_) => {
                self.walk.leave();
                self.pids.dropped = true;
            }
            // Text before the typed value is whitespace; keep waiting.
            _ => self.mode = Mode::Held(property, value),
        }
        Ok(())
    }

    /// Read `event` outside a custom property dropped or held, inside a field
    /// being cleared or not.
    fn copy(&mut self, namespace: Option<&str>, event: Event<'_>) -> io::Result<()> {
        match event {
            Event::Start(elem) => {
                let field = self.walk.enter(namespace, &elem);
                if let Mode::Clearing(depth) = &mut self.mode {
                    *depth += 1;
                    return self.write(Event::Start(elem));
                }
                self.start(field, elem)
            }
            Event::Empty(elem) => {
                self.walk.enter(namespace, &elem);
                let elem = if self.walk.is_property() {
                    self.pids.number(elem.into_owned())
                } else {
                    elem
                };
                self.walk.leave();
                self.write(Event::Empty(elem))
            }
            Event::End(elem) => {
                self.walk.leave();
                if let Mode::Clearing(depth) = &mut self.mode {
                    *depth -= 1;
                    if *depth == 0 {
                        self.mode = Mode::Copy;
                    }
                }
                self.write(Event::End(elem))
            }
            // Text, CDATA, and entity references are field text: kept outside a
            // field being cleared and dropped inside one, so no part of a
            // cleared value (a CDATA section, a `Jos&#233;`) is left behind.
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_)
                if matches!(self.mode, Mode::Clearing(_)) =>
            {
                Ok(())
            }
            other => self.write(other),
        }
    }

    /// Write the start tag `elem` of an element holding `field`, outside a
    /// field being cleared.
    fn start(&mut self, field: Option<Entered>, elem: BytesStart<'_>) -> io::Result<()> {
        let Some(field) = field else {
            return self.start_as_is(elem);
        };
        let value = self.value_of(&field.key);
        let name = (field.place == Place::Custom)
            .then(|| self.value_of(&field.name_key()))
            .flatten();
        if value.is_none() && name.is_none() {
            return self.start_as_is(elem);
        }
        match field.place {
            // A custom property needs a name and a value: a blank one drops
            // it whole.
            Place::Custom if value == Some("") || name == Some("") => self.drop_property(1),
            Place::Custom => {
                let elem = match name {
                    Some(name) => renamed(&elem, name),
                    None => elem.into_owned(),
                };
                match value {
                    Some(value) => self.mode = Mode::Held(elem, value),
                    None => {
                        let elem = self.pids.number(elem);
                        self.write(Event::Start(elem))?;
                    }
                }
            }
            Place::Element | Place::Entry => {
                let value = value.unwrap_or_default();
                self.write(Event::Start(elem))?;
                self.mode = Mode::Clearing(1);
                if !value.is_empty() && !field.timestamp {
                    self.write(Event::Text(BytesText::new(value)))?;
                }
            }
        }
        Ok(())
    }

    /// Write the start tag `elem` as it is: a custom property written as it
    /// is takes its number now; one edited, once it is known to stay.
    fn start_as_is(&mut self, elem: BytesStart<'_>) -> io::Result<()> {
        let elem = if self.walk.is_property() {
            self.pids.number(elem.into_owned())
        } else {
            elem
        };
        self.write(Event::Start(elem))
    }
}

/// `elem` with its `name` attribute set to `name`, every other attribute as it
/// was.
fn renamed(elem: &BytesStart<'_>, name: &str) -> BytesStart<'static> {
    with_attribute(elem, "name", name)
}

/// The pids of the custom properties [`rewrite`] writes: each as written
/// until a property is dropped, then each after it numbered past every pid
/// before it, so the numbering closes over the gap and no pid repeats.
#[derive(Debug, Default)]
struct Pids {
    /// Whether a property has been dropped.
    dropped: bool,
    /// The highest pid written.
    highest: usize,
}

impl Pids {
    /// The custom property `elem`, numbered; one without a pid is as it was.
    fn number(&mut self, elem: BytesStart<'static>) -> BytesStart<'static> {
        let Some(written) = elem.try_get_attribute("pid").ok().flatten() else {
            return elem;
        };
        let written = attribute_value(&written.value)
            .ok()
            .and_then(|pid| pid.parse().ok());
        if !self.dropped {
            self.highest = self.highest.max(written.unwrap_or(0));
            return elem;
        }
        self.highest = self.highest.max(FIRST_PID - 1) + 1;
        if written == Some(self.highest) {
            elem
        } else {
            with_attribute(&elem, "pid", &self.highest.to_string())
        }
    }
}

/// `elem` with its unqualified attribute `key` set to `value`, every other
/// attribute as it was.
fn with_attribute(elem: &BytesStart<'_>, key: &str, value: &str) -> BytesStart<'static> {
    let mut edited = BytesStart::new(elem.name().as_ref().to_owned());
    for attribute in elem.attributes().flatten() {
        if attribute.key.as_ref() == key {
            edited.push_attribute((key, value));
        } else {
            edited.push_attribute(attribute);
        }
    }
    edited
}

/// `bytes` as `&str`, split into the leading byte-order mark some Office parts
/// carry (empty when absent) and the text after it, or `None` when the bytes
/// are not valid UTF-8 so a caller can fail closed rather than silently emit
/// nothing.
fn split_bom(bytes: &[u8]) -> Option<(&str, &str)> {
    let raw = std::str::from_utf8(bytes).ok()?;
    Some(raw.split_at(bom_len(raw)))
}

/// Whether `elem` is a typed value holding text (`vt:lpwstr` and the like),
/// matched by local name.
fn is_text_type(elem: &BytesStart<'_>) -> bool {
    TEXT_TYPES.contains(&elem.local_name().into_inner())
}

/// Resolve an entity reference to the text it stands for, so an entity-encoded
/// field value is read as the character a reader would see.
///
/// A numeric reference (`&#233;`, `&#xE9;`) resolves directly to its code point.
/// A named reference resolves as quick-xml's unescaper resolves it, which knows
/// the five XML predefined entities (`amp`, `lt`, `gt`, `quot`, `apos`). An
/// unknown named entity (a DTD-defined one, which OOXML property parts do not
/// declare) has no value here and contributes nothing, which is the safe
/// reading: it cannot smuggle text past detection.
fn resolve_ref(r: &BytesRef<'_>) -> String {
    if let Ok(Some(ch)) = r.resolve_char_ref() {
        return ch.to_string();
    }
    resolve_predefined_entity(r.as_ref())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::property_fields as pairs;
    use crate::opc::vocabulary::UnreviewedNamespaces;
    use crate::opc::xml_edit::{edit, expect_edit};

    const CORE: &[u8] = br#"<?xml version="1.0" encoding="utf-8"?>
<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties">
  <dc:creator>Ada Lovelace</dc:creator>
  <lastModifiedBy>Alan Turing</lastModifiedBy>
  <dc:title></dc:title>
  <revision>7</revision>
</coreProperties>"#;

    #[test]
    fn reads_populated_sensitive_fields_only() {
        let got = pairs(CORE);
        assert!(got.contains(&("creator".to_owned(), "Ada Lovelace".to_owned())));
        assert!(got.contains(&("lastModifiedBy".to_owned(), "Alan Turing".to_owned())));
        // Empty title and non-sensitive revision are not surfaced.
        assert!(!got.iter().any(|(k, _)| k == "title" || k == "revision"));
    }

    #[test]
    fn redacting_keeps_only_the_surfaced_fields() {
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jane Roe</dc:creator><dc:title>Plan</dc:title><cp:keywords>acquisition</cp:keywords><cp:revision>7</cp:revision><cp:contentStatus>Draft</cp:contentStatus></cp:coreProperties>"#;
        let edited = expect_edit(core, &UnsurfacedProperties);
        assert_eq!(edited.count, 2);
        for kept in ["Jane Roe", "Plan", "acquisition"] {
            assert!(edited.text.contains(kept), "{kept} was stripped");
        }
        assert!(!edited.text.contains("revision") && !edited.text.contains("Draft"));

        let app = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Template>C:\Users\jroe\Normal.dotm</Template><Company>Acme</Company><HyperlinkBase>https://intranet</HyperlinkBase><HeadingPairs><vt:vector size="2" baseType="variant"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>1</vt:i4></vt:variant></vt:vector></HeadingPairs><TitlesOfParts><vt:vector size="1" baseType="lpstr"><vt:lpstr>Secret sheet</vt:lpstr></vt:vector></TitlesOfParts><Application>Microsoft Excel</Application></Properties>"#;
        let edited = expect_edit(app, &UnsurfacedProperties);
        // The application; the heading keeps its count of the one title.
        assert_eq!(edited.count, 1);
        for kept in ["jroe", "Acme", "intranet", "Worksheets", "Secret sheet"] {
            assert!(edited.text.contains(kept), "{kept} was stripped");
        }
        assert!(!edited.text.contains("Excel"));
        assert!(
            edited
                .text
                .contains(r#"<vt:vector size="2" baseType="variant">"#)
        );
        assert!(edited.text.contains("<vt:i4>1</vt:i4>"));

        let custom = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property></Properties>"#;
        assert!(UnsurfacedProperties.apply(custom).unwrap().is_none());

        // A custom property with no name cannot be surfaced, so it goes.
        let unnamed = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2"><vt:lpwstr>Jane Roe</vt:lpwstr></property></Properties>"#;
        assert!(fields(unnamed.as_bytes()).is_empty());
        let edited = expect_edit(unnamed, &UnsurfacedProperties);
        assert_eq!(edited.count, 1);
        assert!(!edited.text.contains("Jane Roe"));
    }

    /// A document type declaration is refused before the root and inside an
    /// element being removed alike.
    #[test]
    fn stripping_refuses_a_document_type() {
        let declared = r#"<!DOCTYPE d [<!ENTITY x "Jane">]><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"/>"#;
        assert!(UnsurfacedProperties.apply(declared).is_err());
        let nested = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"><cp:revision><!DOCTYPE d></cp:revision></cp:coreProperties>"#;
        assert!(UnsurfacedProperties.apply(nested).is_err());
    }

    #[test]
    fn strict_property_parts_keep_only_the_surfaced_fields() {
        let app = r#"<Properties xmlns="http://purl.oclc.org/ooxml/officeDocument/extendedProperties" xmlns:vt="http://purl.oclc.org/ooxml/officeDocument/docPropsVTypes"><Company>Acme</Company><TitlesOfParts><vt:vector size="1" baseType="lpstr"><vt:lpstr>Secret sheet</vt:lpstr></vt:vector></TitlesOfParts><Application>Microsoft Excel</Application></Properties>"#;
        let edited = expect_edit(app, &UnsurfacedProperties);
        assert_eq!(edited.count, 1);
        assert!(edited.text.contains("Acme") && edited.text.contains("Secret sheet"));
        assert!(!edited.text.contains("Excel"));

        let custom = r#"<Properties xmlns="http://purl.oclc.org/ooxml/officeDocument/customProperties" xmlns:vt="http://purl.oclc.org/ooxml/officeDocument/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3"><vt:lpwstr>Unnamed</vt:lpwstr></property></Properties>"#;
        let edited = expect_edit(custom, &UnsurfacedProperties);
        assert_eq!(edited.count, 1);
        assert!(edited.text.contains("Jane Roe"));
        assert!(!edited.text.contains("Unnamed"));
    }

    /// Every piece of text `xml` carries that no field of it reads: text,
    /// CDATA, an entity reference, or a processing instruction outside a
    /// surfaced field, or anything a field holds that its value leaves out.
    /// A heading's count is structure, written from the titles it groups.
    fn unread(xml: &str) -> Vec<String> {
        let read: Vec<String> = fields(xml.as_bytes())
            .into_iter()
            .map(|field| field.value)
            .collect();
        let mut reader = NsReader::from_str(xml);
        let mut unread = Vec::new();
        let mut walk = Walk::default();
        loop {
            let (resolved, event) = reader.read_resolved_event().unwrap();
            let namespace = bound(&resolved);
            match &event {
                Event::Start(e) => {
                    walk.enter(namespace.as_deref(), e);
                }
                Event::End(_) => walk.leave(),
                Event::Text(_) if walk.is_count() => continue,
                _ => {}
            }
            let text = match event {
                Event::Eof => break,
                Event::Text(t) => unescape(t.as_ref()).unwrap().into_owned(),
                Event::CData(t) => t.as_ref().to_owned(),
                Event::GeneralRef(r) => match resolve_ref(&r) {
                    resolved if resolved.is_empty() => format!("&{};", r.as_ref() as &str),
                    resolved => resolved,
                },
                Event::PI(pi) => pi.as_ref().to_owned(),
                _ => continue,
            };
            let text = text.trim();
            if !text.is_empty() && !read.iter().any(|value| value.contains(text)) {
                unread.push(text.to_owned());
            }
        }
        unread
    }

    /// `raw` with its unsurfaced properties removed, asserting that what is
    /// left is all read and that removing again changes nothing.
    fn surfaced_only(raw: &str) -> String {
        let text = UnsurfacedProperties
            .apply(raw)
            .unwrap()
            .map_or_else(|| raw.to_owned(), |edited| edited.text);
        assert_eq!(unread(&text), Vec::<String>::new(), "{text}");
        let vocabulary = UnreviewedNamespaces::default();
        edit(&text, &vocabulary).unwrap();
        assert_eq!(vocabulary.unknown(), None, "{text}");
        assert!(
            UnsurfacedProperties.apply(&text).unwrap().is_none(),
            "a second removal changes {text}"
        );
        text
    }

    const EXTENDED: &str = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">"#;

    #[test]
    fn a_list_entry_outside_a_vector_is_removed() {
        let app = format!(
            "{EXTENDED}<TitlesOfParts><vt:lpstr>Jane Roe salary</vt:lpstr></TitlesOfParts><Company>Acme</Company></Properties>"
        );
        let text = surfaced_only(&app);
        assert!(
            !text.contains("Jane Roe") && !text.contains("TitlesOfParts"),
            "{text}"
        );
        assert!(text.contains("Acme"), "{text}");
    }

    #[test]
    fn a_heading_entry_of_no_text_type_is_removed() {
        let app = format!(
            r#"{EXTENDED}<HeadingPairs><vt:vector size="2" baseType="variant"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>Jane Roe</vt:i4></vt:variant></vt:vector></HeadingPairs></Properties>"#
        );
        let text = surfaced_only(&app);
        assert!(!text.contains("Jane Roe"), "{text}");
        assert!(
            text.contains("Worksheets") && text.contains("<vt:i4>0</vt:i4>"),
            "{text}"
        );
    }

    /// Each heading keeps its count, written as the titles it groups: the
    /// titles kept, taken in order, the last heading taking the rest.
    #[test]
    fn a_heading_keeps_a_count_of_the_titles_it_groups() {
        let app = |counts: [&str; 2], titles: &str| {
            format!(
                r#"{EXTENDED}<HeadingPairs><vt:vector size="4" baseType="variant"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>{}</vt:i4></vt:variant><vt:variant><vt:lpstr>Charts</vt:lpstr></vt:variant><vt:variant><vt:i4>{}</vt:i4></vt:variant></vt:vector></HeadingPairs><TitlesOfParts><vt:vector size="3" baseType="lpstr">{titles}</vt:vector></TitlesOfParts></Properties>"#,
                counts[0], counts[1]
            )
        };
        let titles = "<vt:lpstr>Payroll</vt:lpstr><vt:lpstr/><vt:lpstr>Chart 1</vt:lpstr>";
        assert!(
            UnsurfacedProperties
                .apply(&app(["2", "1"], titles))
                .unwrap()
                .is_none()
        );
        for (counts, kept) in [
            (["555123456", "1"], ["3", "0"]),
            (["Jane Roe", "-4"], ["0", "3"]),
            (["1", ""], ["1", "2"]),
        ] {
            let text = surfaced_only(&app(counts, titles));
            assert_eq!(text, app(kept, titles));
        }
        let empty = format!(
            r#"{EXTENDED}<HeadingPairs><vt:vector size="2" baseType="variant"><vt:variant><vt:lpstr>Pages</vt:lpstr></vt:variant><vt:variant><vt:i4/></vt:variant></vt:vector></HeadingPairs></Properties>"#
        );
        assert!(surfaced_only(&empty).contains("<vt:i4>0</vt:i4>"));
    }

    /// A field is its name in its own vocabulary: the same name in another
    /// namespace is no field, and an element in an unreviewed namespace, or
    /// in none, is neither read nor kept, even inside a field.
    #[test]
    fn a_field_is_matched_by_its_namespace() {
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:v="urn:vendor"><v:title>Jane Roe</v:title><cp:title>John Doe</cp:title><dc:title>Plan</dc:title><dc:subject>a<v:x>Ada</v:x>b</dc:subject><dc:description>c<note xmlns="">Jane</note>d</dc:description></cp:coreProperties>"#;
        assert_eq!(
            pairs(core.as_bytes()),
            [
                ("title".to_owned(), "Plan".to_owned()),
                ("subject".to_owned(), "ab".to_owned()),
                ("description".to_owned(), "cd".to_owned()),
            ]
        );
        let text = surfaced_only(core);
        assert_eq!(
            text,
            r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Plan</dc:title><dc:subject>ab</dc:subject><dc:description>cd</dc:description></cp:coreProperties>"#
        );
        let app = format!(
            r#"{EXTENDED}<Company xmlns="urn:vendor">Acme</Company><TitlesOfParts><vt:vector size="1" baseType="lpstr"><lpstr>Jane</lpstr></vt:vector></TitlesOfParts></Properties>"#
        );
        assert!(pairs(app.as_bytes()).is_empty());
        let text = surfaced_only(&app);
        assert!(!text.contains("Acme") && !text.contains("Jane"), "{text}");
    }

    /// A count reads as XML reads it, however its digits are written.
    #[test]
    fn a_heading_count_reads_its_character_data() {
        let app = |count: &str| {
            format!(
                r#"{EXTENDED}<HeadingPairs><vt:vector size="4" baseType="variant"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>{count}</vt:i4></vt:variant><vt:variant><vt:lpstr>Charts</vt:lpstr></vt:variant><vt:variant><vt:i4>1</vt:i4></vt:variant></vt:vector></HeadingPairs><TitlesOfParts><vt:vector size="2" baseType="lpstr"><vt:lpstr>Payroll</vt:lpstr><vt:lpstr>Chart 1</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#
            )
        };
        for count in ["<![CDATA[1]]>", "&#49;", " 1 "] {
            assert_eq!(surfaced_only(&app(count)), app("1"), "{count}");
        }
    }

    /// Where a cut joins text into a `]]>`, its `>` is written as a
    /// reference, the value read as before.
    #[test]
    fn a_cut_joining_text_keeps_it_well_formed() {
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>a]]<?pi x?>>b</dc:title></cp:coreProperties>"#;
        let text = surfaced_only(core);
        assert!(!text.contains("]]>"), "{text}");
        assert_eq!(
            pairs(text.as_bytes()),
            [("title".to_owned(), "a]]>b".to_owned())]
        );
    }

    #[test]
    fn a_field_named_element_inside_a_list_is_removed() {
        let app = format!(
            "{EXTENDED}<TitlesOfParts><Company>Secret</Company></TitlesOfParts></Properties>"
        );
        let text = surfaced_only(&app);
        assert!(!text.contains("Secret"), "{text}");
    }

    #[test]
    fn a_variant_in_core_properties_is_removed() {
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><dc:creator>Ada</dc:creator><vt:lpwstr>Jane Roe</vt:lpwstr></cp:coreProperties>"#;
        let text = surfaced_only(core);
        assert!(!text.contains("Jane Roe") && text.contains("Ada"), "{text}");
    }

    #[test]
    fn text_outside_every_field_is_removed() {
        let app = format!(
            r#"{EXTENDED}Jane<?note Roe?>
  <Company>Acme</Company><TitlesOfParts>Doe<![CDATA[Smith]]>&#74;<vt:vector size="1" baseType="lpstr"><vt:lpstr>Payroll</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#
        );
        let text = surfaced_only(&app);
        for gone in ["Jane", "Roe", "Doe", "Smith", "&#74;"] {
            assert!(!text.contains(gone), "{gone} kept: {text}");
        }
        assert!(text.contains("Acme") && text.contains("Payroll"), "{text}");
        // The whitespace between fields stays.
        assert!(text.contains("\n  <Company>"), "{text}");
    }

    #[test]
    fn an_entity_no_field_resolves_is_removed() {
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Ada &JaneRoe; &amp; co</dc:creator></cp:coreProperties>"#;
        let text = surfaced_only(core);
        assert!(!text.contains("JaneRoe") && text.contains("Ada"), "{text}");
    }

    #[test]
    fn a_custom_property_keeps_its_value_and_loses_an_empty_one() {
        let custom = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="Carol Doe"/><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="4" name="Count"><vt:i4>42</vt:i4></property></Properties>"#;
        let text = surfaced_only(custom);
        assert!(text.contains("<vt:lpwstr>Jane Roe</vt:lpwstr>"), "{text}");
        assert!(text.contains("<vt:i4>42</vt:i4>"), "{text}");
        assert!(!text.contains("Carol Doe"), "{text}");
    }

    const CUSTOM_ROOT: &str = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes""#;

    #[test]
    fn an_attribute_nothing_reads_is_removed() {
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:v="urn:Jane-Roe" owner="Jane Roe"><dc:creator note="Jane Roe" xml:lang="en-US" xml:space="preserve">Ada</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">2024-01-01T00:00:00Z</dcterms:created><dcterms:modified xsi:type="Jane:Roe">2024-01-01T00:00:00Z</dcterms:modified><dc:title xsi:type="dcterms:W3CDTF">Plan</dc:title></cp:coreProperties>"#;
        let text = surfaced_only(core);
        for gone in ["Jane", "Roe", "note=", "xml:lang", "xml:space", "owner"] {
            assert!(!text.contains(gone), "{gone} kept: {text}");
        }
        assert!(
            text.contains(r#"<dcterms:created xsi:type="dcterms:W3CDTF">"#),
            "{text}"
        );
        assert!(text.contains("<dc:title>Plan</dc:title>"), "{text}");
        assert!(text.contains("<dcterms:modified>"), "{text}");
    }

    #[test]
    fn a_custom_property_keeps_only_its_structural_attributes() {
        let custom = format!(
            r#"{CUSTOM_ROOT}><property fmtid="{{D5CDD505-2E9C-101B-9397-08002B2CF9AE}}" pid="2" name="Client" linkTarget="Jane Roe"><vt:lpwstr>Ada</vt:lpwstr></property><property fmtid="{{4A414E45-524F-4500-0000-000000000000}}" pid="555123456" name="Team"><vt:vector size="987654321" baseType="lpwstr" extra="Roe"><vt:lpwstr>a</vt:lpwstr><vt:lpwstr>b</vt:lpwstr></vt:vector></property><property fmtid="{{D5CDD505-2E9C-101B-9397-08002B2CF9AE}}" pid="4" name="Grid"><vt:array lBounds="0" uBounds="555123456" baseType="i4"><vt:i4>1</vt:i4></vt:array></property></Properties>"#
        );
        let text = surfaced_only(&custom);
        for gone in [
            "Jane",
            "Roe",
            "4A414E45",
            "555123456",
            "987654321",
            "vt:array",
        ] {
            assert!(!text.contains(gone), "{gone} kept: {text}");
        }
        assert_eq!(
            text.matches(r#"fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}""#)
                .count(),
            3,
            "{text}"
        );
        for kept in [
            r#"pid="2" name="Client">"#,
            r#"pid="3" name="Team">"#,
            r#"pid="4" name="Grid">"#,
            r#"<vt:vector size="2" baseType="lpwstr">"#,
        ] {
            assert!(text.contains(kept), "{kept} missing: {text}");
        }
    }

    #[test]
    fn a_list_vector_keeps_a_known_base_type_only() {
        let app = format!(
            r#"{EXTENDED}<TitlesOfParts note="Jane Roe"><vt:vector size="1" baseType="Jane"><vt:lpstr>Payroll</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#
        );
        let text = surfaced_only(&app);
        assert!(!text.contains("Jane"), "{text}");
        assert!(text.contains(r#"<vt:vector size="1">"#), "{text}");
    }

    #[test]
    fn empty_list_entries_keep_their_vector() {
        let app = format!(
            r#"{EXTENDED}<TitlesOfParts><vt:vector size="2" baseType="lpstr"><vt:lpstr/><vt:lpstr>Payroll</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#
        );
        assert!(UnsurfacedProperties.apply(&app).unwrap().is_none());
    }

    const APP: &[u8] = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Template>Normal.dotm</Template><TitlesOfParts><vt:vector size="3" baseType="lpstr"><vt:lpstr>Payroll</vt:lpstr><vt:lpstr/><vt:lpstr>Jane Roe</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#;

    const CUSTOM: &[u8] = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="creator"><vt:i4>42</vt:i4></property></Properties>"#;

    #[test]
    fn each_title_of_parts_is_a_field_by_position() {
        let got = fields(APP);
        let keys: Vec<&str> = got.iter().map(|field| field.key.as_str()).collect();
        assert_eq!(keys, ["Template", "TitlesOfParts/0", "TitlesOfParts/2"]);
        assert_eq!(got[2].value, "Jane Roe");
        assert_eq!(
            got[1].label,
            LabelRef::from(&*builtins::COMMUNICATIONS_CONTENT)
        );
    }

    #[test]
    fn a_removed_title_of_parts_keeps_its_place() {
        let out = rewrite(
            APP,
            &[("TitlesOfParts/2", ""), ("TitlesOfParts/0", "[TITLE]")],
        );
        let text = std::str::from_utf8(&out).unwrap();
        assert!(!text.contains("Jane Roe") && !text.contains("Payroll"));
        assert_eq!(text.matches("<vt:lpstr").count(), 3, "{text}");
        assert_eq!(
            pairs(&out),
            [
                ("Template".to_owned(), "Normal.dotm".to_owned()),
                ("TitlesOfParts/0".to_owned(), "[TITLE]".to_owned()),
            ]
        );
    }

    #[test]
    fn a_custom_property_is_keyed_by_position_with_its_name_a_field() {
        let got = fields(CUSTOM);
        assert_eq!(
            pairs(CUSTOM),
            [
                ("property/0/name".to_owned(), "Client".to_owned()),
                ("property/0".to_owned(), "Jane Roe".to_owned()),
                ("property/1/name".to_owned(), "creator".to_owned()),
                ("property/1".to_owned(), "42".to_owned()),
            ]
        );
        // Named like a core field, a custom property is still free content.
        assert!(
            got.iter()
                .all(|field| field.label == LabelRef::from(&*builtins::COMMUNICATIONS_CONTENT))
        );
    }

    #[test]
    fn each_heading_is_a_field_beside_the_titles() {
        let app = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><HeadingPairs><vt:vector size="2" baseType="variant"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>1</vt:i4></vt:variant></vt:vector></HeadingPairs><TitlesOfParts><vt:vector size="1" baseType="lpstr"><vt:lpstr>Payroll</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#;
        assert_eq!(
            pairs(app),
            [
                ("HeadingPairs/0".to_owned(), "Worksheets".to_owned()),
                ("TitlesOfParts/0".to_owned(), "Payroll".to_owned()),
            ]
        );
    }

    #[test]
    fn a_custom_list_reads_one_entry_per_line_and_is_dropped_when_edited() {
        let custom = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Reviewers"><vt:vector size="2" baseType="lpwstr"><vt:lpwstr>Jane Roe</vt:lpwstr><vt:lpwstr>carol@example.com</vt:lpwstr></vt:vector></property></Properties>"#;
        assert_eq!(
            pairs(custom),
            [
                ("property/0/name".to_owned(), "Reviewers".to_owned()),
                (
                    "property/0".to_owned(),
                    "Jane Roe\ncarol@example.com".to_owned()
                ),
            ]
        );
        // A list cannot hold the edited text, so the property goes whole.
        let out = rewrite(custom, &[("property/0", "Jane Roe\n[EMAIL]")]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            !text.contains("Reviewers") && !text.contains("carol"),
            "{text}"
        );
    }

    #[test]
    fn a_repeated_name_is_its_own_field() {
        // Custom properties are keyed by position, so a repeated name, or one
        // that looks like another key, never collides.
        let custom = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="Client"><vt:lpwstr>ACME</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="4" name="property/0"><vt:lpwstr>Kyiv</vt:lpwstr></property></Properties>"#;
        let out = rewrite(custom, &[("property/0", "[NAME]")]);
        let after = pairs(&out);
        assert!(after.contains(&("property/0".to_owned(), "[NAME]".to_owned())));
        assert!(after.contains(&("property/1".to_owned(), "ACME".to_owned())));
        assert!(after.contains(&("property/2".to_owned(), "Kyiv".to_owned())));

        let core = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Ada</dc:creator><dc:creator>Alan</dc:creator></coreProperties>"#;
        let got = fields(core);
        assert_eq!(got[1].key, "creator#2");
        assert_eq!(got[1].label, LabelRef::from(&*builtins::PERSON_NAME));
    }

    #[test]
    fn a_custom_property_is_renamed_or_dropped_with_its_name() {
        let out = rewrite(CUSTOM, &[("property/0/name", "[NAME]")]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            text.contains(r#"name="[NAME]""#) && !text.contains("Client"),
            "{text}"
        );
        assert!(text.contains("Jane Roe"), "{text}");

        let out = rewrite(CUSTOM, &[("property/0/name", "")]);
        assert_eq!(
            pairs(&out),
            [
                ("property/0/name".to_owned(), "creator".to_owned()),
                ("property/0".to_owned(), "42".to_owned()),
            ]
        );
    }

    #[test]
    fn a_custom_property_is_overwritten_in_its_text_or_dropped() {
        let out = rewrite(
            CUSTOM,
            &[("property/0", "[NAME]"), ("property/1", "[NAME]")],
        );
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            text.contains(r#"name="Client"><vt:lpwstr>[NAME]</vt:lpwstr></property>"#),
            "{text}"
        );
        // A number cannot hold the text, so the property goes whole.
        assert!(!text.contains("creator") && !text.contains("42"), "{text}");

        let out = rewrite(CUSTOM, &[("property/0", "")]);
        assert_eq!(
            pairs(&out),
            [
                ("property/0/name".to_owned(), "creator".to_owned()),
                ("property/0".to_owned(), "42".to_owned()),
            ]
        );
    }

    /// A custom property dropped by a redaction leaves the rest numbered by
    /// position, so the part needs no removal after.
    #[test]
    fn a_dropped_custom_property_leaves_the_rest_numbered() {
        let custom = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="Team"><vt:lpwstr>Ops</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="4" name="Site"><vt:lpwstr>Kyiv</vt:lpwstr></property></Properties>"#;
        let raw = std::str::from_utf8(custom).unwrap();
        assert!(UnsurfacedProperties.apply(raw).unwrap().is_none());
        for edits in [
            &[("property/0", "")][..],
            &[("property/1/name", "")],
            &[("property/0", ""), ("property/2/name", "[NAME]")],
        ] {
            let out = rewrite(custom, edits);
            let text = std::str::from_utf8(&out).unwrap();
            assert!(
                text.contains(r#"pid="2""#) && text.contains(r#"pid="3""#),
                "{text}"
            );
            assert!(!text.contains(r#"pid="4""#), "{text}");
            assert!(
                UnsurfacedProperties.apply(text).unwrap().is_none(),
                "{edits:?}: {text}"
            );
        }
    }

    /// An empty custom property keeps a pid of its own beside a renumbered
    /// one, and an edit naming no field leaves the pids as written.
    #[test]
    fn custom_property_pids_never_repeat_and_stay_put_without_a_drop() {
        let custom = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="7" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property><property pid="2" name="A"/><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="5" name="B"><vt:lpwstr>x</vt:lpwstr></property></Properties>"#;
        let out = rewrite(custom, &[("nothing", "x")]);
        assert_eq!(std::str::from_utf8(&out), std::str::from_utf8(custom));
        let out = rewrite(custom, &[("property/0", "")]);
        let text = std::str::from_utf8(&out).unwrap();
        let mut pids: Vec<&str> = text
            .split(r#"pid=""#)
            .skip(1)
            .map(|rest| rest.split('"').next().unwrap())
            .collect();
        assert_eq!(pids.len(), 2, "{text}");
        pids.dedup();
        assert_eq!(pids.len(), 2, "{text}");
    }

    /// An overwritten custom property with no typed value is dropped, and
    /// the next one numbered into its place.
    #[test]
    fn an_overwritten_property_without_a_value_leaves_no_gap() {
        let custom = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="Team"><vt:lpwstr>Ops</vt:lpwstr></property></Properties>"#;
        let out = rewrite(custom, &[("property/0", "x")]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(!text.contains("Client"), "{text}");
        assert!(text.contains(r#"pid="2" name="Team""#), "{text}");
        assert!(
            UnsurfacedProperties.apply(text).unwrap().is_none(),
            "{text}"
        );
    }

    #[test]
    fn rewrite_overwrites_a_text_field_and_clears_a_timestamp() {
        let xml = br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/"><dc:creator>Ada &amp; co</dc:creator><dcterms:created>2024-01-02T03:04:05Z</dcterms:created></cp:coreProperties>"#;
        let out = rewrite(xml, &[("creator", "[A & B]"), ("created", "never")]);
        let after = pairs(&out);
        assert!(after.contains(&("creator".to_owned(), "[A & B]".to_owned())));
        assert!(!after.iter().any(|(k, _)| k == "created"));
        assert!(!std::str::from_utf8(&out).unwrap().contains("never"));
    }

    #[test]
    fn strip_clears_named_fields_and_keeps_the_rest() {
        let out = rewrite(CORE, &[("creator", "")]);
        let after = pairs(&out);
        // creator is gone, lastModifiedBy remains.
        assert!(!after.iter().any(|(k, _)| k == "creator"));
        assert!(
            after
                .iter()
                .any(|(k, v)| k == "lastModifiedBy" && v == "Alan Turing")
        );
        // The revision element survives in the bytes.
        assert!(
            std::str::from_utf8(&out)
                .unwrap()
                .contains("<revision>7</revision>")
        );
    }

    #[test]
    fn strip_preserves_a_leading_bom() {
        const BOM: &[u8] = "\u{feff}".as_bytes();
        let mut with_bom = BOM.to_vec();
        with_bom.extend_from_slice(CORE);
        let out = rewrite(&with_bom, &[("creator", "")]);
        assert!(out.starts_with(BOM), "BOM dropped");
    }

    #[test]
    fn strip_of_non_utf8_returns_the_original_bytes() {
        // A lone continuation byte is not valid UTF-8: fail closed, don't blank.
        let garbage: &[u8] = &[0x3c, 0xFF, 0x3e];
        let out = rewrite(garbage, &[("creator", "")]);
        assert_eq!(&out[..], garbage, "non-UTF-8 input was not preserved");
    }

    #[test]
    fn a_self_closing_sensitive_element_carries_no_value() {
        // `<dc:creator/>` has no text, so it is neither surfaced nor changed.
        let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator/></coreProperties>"#;
        assert!(pairs(xml).is_empty(), "empty self-closing field surfaced");
        let out = rewrite(xml, &[("creator", "")]);
        assert!(
            std::str::from_utf8(&out).unwrap().contains("creator"),
            "self-closing element should survive structurally"
        );
    }

    #[test]
    fn a_cdata_field_value_is_read_and_stripped() {
        // A value wrapped in CDATA is field text like any other: it must be
        // surfaced for detection and removed on strip, not passed through.
        let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator><![CDATA[Ada Lovelace]]></dc:creator></coreProperties>"#;
        assert!(
            pairs(xml).contains(&("creator".to_owned(), "Ada Lovelace".to_owned())),
            "CDATA field value not surfaced"
        );

        let out = rewrite(xml, &[("creator", "")]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            !text.contains("Ada Lovelace"),
            "CDATA value leaked past strip: {text}"
        );
        assert!(
            text.contains("creator"),
            "the element itself should survive"
        );
    }

    #[test]
    fn a_numeric_entity_reference_is_read_and_stripped() {
        // `José` written with a numeric character reference. quick-xml surfaces
        // the `&#233;` as its own event; the value must still read as `José` for
        // detection, and strip must clear the whole thing including the entity.
        let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jos&#233; Bob</dc:creator></coreProperties>"#;
        assert!(
            pairs(xml).contains(&("creator".to_owned(), "José Bob".to_owned())),
            "numeric-entity value not fully surfaced: {:?}",
            pairs(xml)
        );

        let out = rewrite(xml, &[("creator", "")]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            !text.contains("Bob"),
            "value text leaked past strip: {text}"
        );
        assert!(
            !text.contains("&#233;"),
            "entity ref leaked past strip: {text}"
        );
        assert!(
            text.contains("creator"),
            "the element itself should survive"
        );
    }

    #[test]
    fn a_wholly_entity_encoded_value_is_surfaced() {
        // A value made entirely of character references (`Hi`) must not vanish
        // from detection just because it carries no plain text node.
        let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>&#72;&#105;</dc:creator></coreProperties>"#;
        assert!(
            pairs(xml).contains(&("creator".to_owned(), "Hi".to_owned())),
            "all-entity value not surfaced: {:?}",
            pairs(xml)
        );
    }

    #[test]
    fn a_named_entity_reference_is_read_and_stripped() {
        // A predefined named entity (`&amp;`) inside a value: read as `&` and
        // cleared on strip.
        let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>A &amp; B</dc:creator></coreProperties>"#;
        assert!(
            pairs(xml).contains(&("creator".to_owned(), "A & B".to_owned())),
            "named-entity value not surfaced: {:?}",
            pairs(xml)
        );

        let out = rewrite(xml, &[("creator", "")]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            !text.contains("&amp;"),
            "named entity leaked past strip: {text}"
        );
        assert!(!text.contains(" B"), "value text leaked past strip: {text}");
    }
}
