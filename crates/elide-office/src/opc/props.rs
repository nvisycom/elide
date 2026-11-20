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

use bytes::Bytes;
use elide_core::Result;
use elide_core::entity::{LabelRef, builtins};
use elide_core::redaction::HiddenContent;
use quick_xml::NsReader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::name::QName;
use quick_xml::writer::Writer;

use crate::opc::ns::{
    CORE_PROPERTIES, CUSTOM_PROPERTIES, DC, DCTERMS, DOC_PROPS_VTYPES, EXTENDED_PROPERTIES, XSI,
    transitional,
};
use crate::opc::vocabulary::is_known;
use crate::opc::xml_edit::{
    Edit, Edited, Skipped, SpannedReader, attribute_extent, attribute_source, bom_len, bound,
    local_name, namespace_of,
};
use crate::opc::{attribute_value, malformed_xml};

/// The core and extended property elements surfaced as fields, each holding
/// its value as text.
const ELEMENT_FIELDS: [&str; 13] = [
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
const TIMESTAMPS: [&str; 2] = ["created", "modified"];

/// The variant types (`vt:`) that hold free text: a list entry, or a custom
/// property that can be overwritten. A custom property of any other type (a
/// number, a date, a flag, a list) is removed instead.
const TEXT_TYPES: [&str; 3] = ["lpwstr", "lpstr", "bstr"];

/// The extended properties that list the document's parts: `TitlesOfParts`
/// names each part (a sheet, a slide title), and `HeadingPairs` says how those
/// names group (`Worksheets`, 3). Kept together, since each describes the other.
const LISTS: [&str; 2] = ["TitlesOfParts", "HeadingPairs"];

/// A custom property's element, named by its `name` attribute.
const CUSTOM_PROPERTY: &str = "property";

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
    /// What the field `key` in this place is: an element field by the schema's
    /// meaning for it, any other field free content.
    fn label(self, key: &str) -> LabelRef {
        let label = match (self, key) {
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
pub struct Field {
    /// The field's key: an element's local name, `<list>/<n>`, or a custom
    /// property's `property/<n>` (its name `property/<n>/name`).
    pub key: String,
    /// The field's value, trimmed.
    pub value: String,
    /// What the field is.
    pub label: LabelRef,
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
        let mut walk = Walk::default();
        let mut cuts: Vec<(Range<usize>, Cut)> = Vec::new();
        // The kept elements that are not fields: the root, and the list
        // containers, vectors, and variants, kept only when an entry is.
        let mut frames: Vec<Frame> = Vec::new();
        // The vectors open inside the field being read, each keeping its
        // `size` to the entries it holds.
        let mut vectors: Vec<Frame> = Vec::new();
        // The depth of the surfaced field being read, everything inside kept
        // but an array.
        let mut reading: Option<usize> = None;
        // An element being removed, left out of the walk.
        let mut removing: Option<Skipped> = None;
        // How many custom properties are kept, which numbers each.
        let mut properties = 0usize;
        // The heading counts, written once the titles they group are known.
        let mut counts: Vec<Count<'_>> = Vec::new();
        // A heading count being read: where its content begins, how many
        // elements are open inside it, itself included, and its character
        // data as XML reads it (none once an element or a reference that
        // does not decode is met).
        let mut counting: Option<(usize, usize, Option<String>)> = None;
        // How many titles of parts are kept.
        let mut titles = 0usize;

        loop {
            let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;
            if let Some((start, depth, text)) = &mut counting {
                match event {
                    Event::Start(_) => {
                        *depth += 1;
                        *text = None;
                    }
                    Event::Empty(_) => *text = None,
                    Event::End(_) => *depth -= 1,
                    Event::Eof => return Err(malformed_xml("unclosed element")),
                    Event::Text(ref t) => {
                        let decoded = unescape(t.as_ref()).ok();
                        *text = text.take().zip(decoded).map(|(text, t)| text + &t);
                    }
                    Event::CData(ref t) => {
                        if let Some(text) = text {
                            text.push_str(t.as_ref());
                        }
                    }
                    Event::GeneralRef(ref r) => {
                        let resolved = resolve_ref(r);
                        if resolved.is_empty() {
                            *text = None;
                        } else if let Some(text) = text {
                            text.push_str(&resolved);
                        }
                    }
                    _ => {}
                }
                if *depth == 0 {
                    let said = text.as_deref().and_then(|text| text.trim().parse().ok());
                    counts.push(Count {
                        range: *start..span.start,
                        close: None,
                        said,
                    });
                    counting = None;
                    walk.leave();
                }
                continue;
            }
            if Skipped::skip(&mut removing, &event, span.end, &mut cuts)? {
                continue;
            }
            match event {
                Event::Eof => break,
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let is_start = matches!(event, Event::Start(_));
                    let depth = walk.depth();
                    let field = walk.enter(namespace.as_deref(), e);
                    let local = e.local_name().into_inner();
                    // A heading's count is kept, its value written from the
                    // titles it groups.
                    if reading.is_none() && walk.is_count() {
                        keep_child(&mut frames);
                        let element = Element {
                            namespace: namespace.as_deref().map(transitional),
                            local,
                            sized: false,
                            pid: None,
                        };
                        cut_attributes(&reader.reader, raw, e, &element, &mut cuts)?;
                        if is_start {
                            counting = Some((span.end, 1, Some(String::new())));
                        } else {
                            walk.leave();
                            counts.push(Count {
                                range: span.end - 2..span.end,
                                close: Some(&raw[span.start + 1..][..e.name().as_ref().len()]),
                                said: None,
                            });
                        }
                        continue;
                    }
                    let mut pid = None;
                    let kept = if reading.is_some() {
                        // An array's bounds are numbers nothing reads, and
                        // nothing reads an unreviewed vocabulary.
                        local != "array" && !walk.is_foreign()
                    } else if depth == 0 {
                        true
                    } else if let Some(field) = field {
                        // A custom property's name is read only when it
                        // holds a value, so an empty one is never surfaced.
                        let read = is_start || field.place != Place::Custom;
                        if read {
                            keep_child(&mut frames);
                            if field.place == Place::Custom {
                                properties += 1;
                                pid = Some(properties + 1);
                            }
                            if field.place == Place::Entry && walk.open[1] == "TitlesOfParts" {
                                titles += 1;
                            }
                            if is_start {
                                reading = Some(depth);
                            }
                        }
                        read
                    } else {
                        is_start && walk.is_list_path()
                    };
                    if !kept {
                        walk.leave();
                        removing = Skipped::begin(span, is_start, &mut cuts);
                        continue;
                    }
                    if let Some(vector) = vectors.last_mut()
                        && vector.depth + 1 == depth
                    {
                        vector.children += 1;
                    }
                    let sized = local == "vector";
                    if !is_start {
                        walk.leave();
                    } else if reading.is_some_and(|field| field < depth) {
                        if sized {
                            vectors.push(Frame::new(depth, span.start, cuts.len(), size(raw, e)?));
                        }
                    } else if reading.is_none() {
                        let size = if sized { size(raw, e)? } else { None };
                        frames.push(Frame::new(depth, span.start, cuts.len(), size));
                    }
                    let element = Element {
                        namespace: namespace.as_deref().map(transitional),
                        local,
                        sized: sized && is_start,
                        pid,
                    };
                    cut_attributes(&reader.reader, raw, e, &element, &mut cuts)?;
                }
                Event::End(_) => {
                    walk.leave();
                    let depth = walk.depth();
                    if reading == Some(depth) {
                        reading = None;
                    } else if reading.is_some() {
                        if let Some(vector) = vectors.pop_if(|vector| vector.depth == depth) {
                            vector.resize(&mut cuts);
                        }
                    } else if let Some(frame) = frames.pop_if(|frame| frame.depth == depth) {
                        if frame.children == 0 && depth > 0 {
                            cuts.truncate(frame.cuts);
                            cuts.push((frame.start..span.end, Cut::Removed));
                        } else {
                            frame.resize(&mut cuts);
                            keep_child(&mut frames);
                        }
                    }
                }
                Event::Text(ref text) if reading.is_none() && !is_blank(text) => {
                    cuts.push((span, Cut::Removed));
                }
                Event::CData(_) if reading.is_none() => cuts.push((span, Cut::Removed)),
                Event::GeneralRef(ref r) if reading.is_none() || resolve_ref(r).is_empty() => {
                    cuts.push((span, Cut::Removed));
                }
                Event::PI(_) => cuts.push((span, Cut::Removed)),
                _ => {}
            }
        }
        write_counts(raw, &counts, titles, &mut cuts);
        let count = cuts.iter().filter(|(_, cut)| cut.counted()).count();
        Ok(Edited::from_cuts(raw, cuts, count))
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
fn write_counts(
    raw: &str,
    counts: &[Count<'_>],
    titles: usize,
    cuts: &mut Vec<(Range<usize>, Cut)>,
) {
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
            None => cuts.push((count.range.clone(), Cut::Replaced(value))),
            Some(name) => cuts.push((
                count.range.clone(),
                Cut::Replaced(format!(">{value}</{name}>")),
            )),
        }
    }
}

/// A kept element of a property part, as its attributes are decided.
struct Element<'a> {
    /// Its namespace, a strict one as its transitional twin.
    namespace: Option<&'a str>,
    local: &'a str,
    /// Whether it is a vector whose `size` is kept to its entries.
    sized: bool,
    /// For a kept custom property, the number it is given.
    pid: Option<usize>,
}

/// The format id every custom property a person adds carries
/// (`FMTID_UserDefinedProperties`).
const USER_DEFINED: &str = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}";

/// The variant types a vector or an array may be of.
const BASE_TYPES: &[&str] = &[
    "variant", "i1", "i2", "i4", "i8", "int", "ui1", "ui2", "ui4", "ui8", "uint", "r4", "r8",
    "decimal", "lpstr", "lpwstr", "bstr", "date", "filetime", "bool", "cy", "error", "clsid",
];

/// What a property part keeps of an attribute.
enum Kept {
    /// The attribute as it is.
    As,
    /// The attribute with this value, in place of one that could carry text.
    With(String),
    /// Nothing.
    No,
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
    cuts: &mut Vec<(Range<usize>, Cut)>,
) -> Result<()> {
    for attr in e.attributes() {
        let attr = attr.map_err(malformed_xml)?;
        let (extent, value_range) = attribute_extent(raw, &attr)?;
        let written = &raw[value_range.clone()];
        let value = attribute_value(written).ok();
        let value = value.as_deref();
        let kept = if attr.key.as_namespace_binding().is_some() {
            let reviewed = value.is_some_and(|uri| uri.is_empty() || is_known(transitional(uri)));
            if reviewed { Kept::As } else { Kept::No }
        } else {
            let (resolved, local) = reader.resolver().resolve_attribute(attr.key);
            let qualified = namespace_of(&resolved);
            match (element.local, qualified.as_deref(), local.into_inner()) {
                (_, Some(XSI), "type")
                    if element.namespace == Some(DCTERMS) && is_w3cdtf(reader, value) =>
                {
                    Kept::As
                }
                (_, None, "name") if element.pid.is_some() => Kept::As,
                (_, None, "pid") => match element.pid {
                    Some(pid) if written == pid.to_string() => Kept::As,
                    Some(pid) => Kept::With(pid.to_string()),
                    None => Kept::No,
                },
                (_, None, "fmtid") if element.pid.is_some() => {
                    if written == USER_DEFINED {
                        Kept::As
                    } else {
                        Kept::With(USER_DEFINED.to_owned())
                    }
                }
                // Kept to the vector's entries when it closes.
                (_, None, "size") if element.sized => Kept::As,
                ("vector", None, "baseType") if value.is_some_and(|t| BASE_TYPES.contains(&t)) => {
                    Kept::As
                }
                _ => Kept::No,
            }
        };
        match kept {
            Kept::As => {}
            Kept::With(value) => cuts.push((value_range, Cut::Replaced(value))),
            Kept::No => cuts.push((extent, Cut::Removed)),
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
    let (prefix, local) = value.split_once(':').unwrap_or(("", value));
    if local != "W3CDTF" {
        return false;
    }
    let name = if prefix.is_empty() {
        "x".to_owned()
    } else {
        format!("{prefix}:x")
    };
    let (resolved, _) = reader.resolver().resolve_element(QName(&name));
    namespace_of(&resolved).as_deref() == Some(DCTERMS)
}

/// What an [`UnsurfacedProperties`] edit writes in place of a range: nothing
/// for what it removes, a value in place of one that could carry text (both
/// counted), or a vector's new `size` (not counted).
#[derive(Default)]
enum Cut {
    #[default]
    Removed,
    Replaced(String),
    Written(String),
}

impl Cut {
    /// Whether this cut removes content.
    fn counted(&self) -> bool {
        !matches!(self, Self::Written(_))
    }
}

impl AsRef<str> for Cut {
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
    /// For a vector, its `size` value's range and text.
    size: Option<(Range<usize>, String)>,
    /// How many of its children are kept.
    children: usize,
}

impl Frame {
    fn new(depth: usize, start: usize, cuts: usize, size: Option<(Range<usize>, String)>) -> Self {
        Self {
            depth,
            start,
            cuts,
            size,
            children: 0,
        }
    }
}

impl Frame {
    /// Set the `size` this vector was written with to the children it keeps.
    fn resize(self, cuts: &mut Vec<(Range<usize>, Cut)>) {
        if let Some((value, written)) = self.size
            && written != self.children.to_string()
        {
            cuts.push((value, Cut::Written(self.children.to_string())));
        }
    }
}

/// Count a kept child of the innermost open frame.
fn keep_child(frames: &mut [Frame]) {
    if let Some(frame) = frames.last_mut() {
        frame.children += 1;
    }
}

/// The range and text of `e`'s unqualified `size` attribute, if it has one.
fn size(raw: &str, e: &BytesStart<'_>) -> Result<Option<(Range<usize>, String)>> {
    for attribute in e.attributes() {
        let attribute = attribute.map_err(malformed_xml)?;
        if attribute.key.as_ref() == "size" {
            let (_, value) = attribute_source(raw, &attribute)?;
            return Ok(Some((value.clone(), raw[value].to_owned())));
        }
    }
    Ok(None)
}

/// Whether `text` is only whitespace.
fn is_blank(text: &BytesText<'_>) -> bool {
    text.as_ref().chars().all(|c| c.is_ascii_whitespace())
}

/// A field an element holds: its key in the part, where its value sits, what
/// it is, and for a custom property, its name.
struct Entered {
    key: String,
    place: Place,
    label: LabelRef,
    name: Option<String>,
}

impl Entered {
    /// The key of the field holding a custom property's name.
    fn name_key(&self) -> String {
        format!("{}/name", self.key)
    }
}

/// The open elements of a property part, which say what field an element is.
#[derive(Debug, Default)]
struct Walk {
    /// The local names of the open elements, the root first.
    open: Vec<String>,
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
    /// unreviewed namespace holds nothing read.
    ///
    /// Every key is unique in its part. An element field is keyed by its name,
    /// a repeated one with `#<n>` after it (no element name holds a `#`). A
    /// list entry is keyed `<list>/<n>`, and a custom property by its position,
    /// `property/<n>`, since its name is free text that could look like any
    /// key; the name is a field of its own, `property/<n>/name`.
    fn enter(&mut self, namespace: Option<&str>, elem: &BytesStart<'_>) -> Option<Entered> {
        let namespace = namespace.map(transitional);
        if self.outside.is_some() || namespace.is_some_and(|namespace| !is_known(namespace)) {
            self.outside.get_or_insert(self.open.len());
            self.open.push(String::new());
            return None;
        }
        let local = local_name(elem);
        // A name out of its vocabulary is kept marked, so it matches none.
        let local = match vocabulary(&local) {
            Some(expected) if namespace != Some(expected) => format!("?{local}"),
            _ => local,
        };
        let entered = match self.open.len() {
            1 if ELEMENT_FIELDS.contains(&local.as_str()) => {
                let count = self.seen.entry(local.clone()).or_default();
                *count += 1;
                let key = match *count {
                    1 => local.clone(),
                    n => format!("{local}#{n}"),
                };
                Some(Entered {
                    key,
                    place: Place::Element,
                    label: Place::Element.label(&local),
                    name: None,
                })
            }
            1 if local == CUSTOM_PROPERTY => elem
                .try_get_attribute("name")
                .ok()
                .flatten()
                .and_then(|name| attribute_value(&name.value).ok().map(Cow::into_owned))
                .map(|name| {
                    self.properties += 1;
                    Entered {
                        key: format!("{CUSTOM_PROPERTY}/{}", self.properties - 1),
                        place: Place::Custom,
                        label: Place::Custom.label(&local),
                        name: Some(name),
                    }
                }),
            1 if LISTS.contains(&local.as_str()) => {
                self.entries = 0;
                None
            }
            // A list's entries sit in its vector, a heading's in a variant.
            depth @ (3 | 4)
                if LISTS.contains(&self.open[1].as_str())
                    && self.open[2] == "vector"
                    && (depth == 3 || self.open[3] == "variant")
                    && TEXT_TYPES.contains(&local.as_str()) =>
            {
                self.entries += 1;
                Some(Entered {
                    key: format!("{}/{}", self.open[1], self.entries - 1),
                    place: Place::Entry,
                    label: Place::Entry.label(&local),
                    name: None,
                })
            }
            _ => None,
        };
        self.open.push(local);
        entered
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

    /// Whether the innermost open element is a custom property.
    fn is_property(&self) -> bool {
        self.open.len() == 2 && self.open[1] == CUSTOM_PROPERTY
    }

    /// Whether the innermost open element is a heading's count: an `i4`
    /// variant of the heading pairs' vector.
    fn is_count(&self) -> bool {
        self.open.len() == 5
            && self.open[1] == "HeadingPairs"
            && self.open[2] == "vector"
            && self.open[3] == "variant"
            && self.open[4] == "i4"
    }

    /// Whether the innermost open element is a list container, its vector,
    /// or a variant of that vector: the path to a list entry.
    fn is_list_path(&self) -> bool {
        let is_list = || LISTS.contains(&self.open[1].as_str());
        match self.open.len() {
            2 => is_list(),
            3 => is_list() && self.open[2] == "vector",
            4 => is_list() && self.open[2] == "vector" && self.open[3] == "variant",
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
pub fn fields(xml: &[u8]) -> Vec<Field> {
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
pub fn rewrite(xml: &[u8], fields: &[(&str, &str)]) -> Bytes {
    // Not valid UTF-8: fail closed by preserving the original bytes rather than
    // emitting a blank part. A property part is always UTF-8 XML in practice.
    let Some((bom, text)) = split_bom(xml) else {
        return Bytes::copy_from_slice(xml);
    };
    let value_of = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.trim())
    };
    let mut reader = NsReader::from_str(text);
    reader.config_mut().trim_text(false);
    // The output keeps the part's BOM, written ahead of everything else.
    let mut writer = Writer::new(bom.as_bytes().to_vec());
    let mut walk = Walk::default();
    // Depth inside a field being cleared, so nested text is dropped and the
    // elements re-emit empty.
    let mut clearing = 0u32;
    // Depth inside a custom property being dropped, so nothing of it is emitted.
    let mut dropping = 0u32;
    // A custom property being overwritten, held back until its typed child
    // says whether the new value fits.
    let mut held: Option<(BytesStart<'static>, &str)> = None;
    // The pids of the custom properties written.
    let mut pids = Pids::default();

    loop {
        let (namespace, event) = match reader.read_resolved_event() {
            Ok((_, Event::Eof)) => break,
            Ok((resolved, event)) => (bound(&resolved), event),
            Err(_) => return Bytes::copy_from_slice(xml),
        };
        if dropping > 0 {
            match event {
                Event::Start(elem) => {
                    walk.enter(namespace.as_deref(), &elem);
                    dropping += 1;
                }
                Event::End(_) => {
                    walk.leave();
                    dropping -= 1;
                }
                _ => {}
            }
            continue;
        }
        if let Some((property, value)) = held.take() {
            match event {
                Event::Start(elem) if is_text_type(&elem) => {
                    walk.enter(namespace.as_deref(), &elem);
                    let property = pids.number(property);
                    let _ = writer.write_event(Event::Start(property));
                    let _ = writer.write_event(Event::Start(elem));
                    let _ = writer.write_event(Event::Text(BytesText::new(value)));
                    clearing = 2;
                }
                Event::Start(elem) => {
                    walk.enter(namespace.as_deref(), &elem);
                    dropping = 2;
                    pids.dropped = true;
                }
                Event::Empty(_) => {
                    dropping = 1;
                    pids.dropped = true;
                }
                // A property with no typed value goes.
                Event::End(_) => {
                    walk.leave();
                    pids.dropped = true;
                }
                // Text before the typed value is whitespace; keep waiting.
                _ => held = Some((property, value)),
            }
            continue;
        }
        match event {
            Event::Start(elem) => {
                let field = walk.enter(namespace.as_deref(), &elem);
                if clearing > 0 {
                    clearing += 1;
                    let _ = writer.write_event(Event::Start(elem));
                    continue;
                }
                // A custom property written as it is takes its number now;
                // one edited, once it is known to stay.
                let as_is = |elem: BytesStart<'_>, pids: &mut Pids| {
                    if walk.is_property() {
                        pids.number(elem.into_owned())
                    } else {
                        elem.into_owned()
                    }
                };
                let Some(field) = field else {
                    let _ = writer.write_event(Event::Start(as_is(elem, &mut pids)));
                    continue;
                };
                let value = value_of(&field.key);
                let name = (field.place == Place::Custom)
                    .then(|| value_of(&field.name_key()))
                    .flatten();
                if value.is_none() && name.is_none() {
                    let _ = writer.write_event(Event::Start(as_is(elem, &mut pids)));
                    continue;
                }
                match field.place {
                    // A custom property needs a name and a value: a blank one
                    // drops it whole.
                    Place::Custom if value == Some("") || name == Some("") => {
                        dropping = 1;
                        pids.dropped = true;
                    }
                    Place::Custom => {
                        let elem = match name {
                            Some(name) => renamed(&elem, name),
                            None => elem.into_owned(),
                        };
                        match value {
                            Some(value) => held = Some((elem, value)),
                            None => {
                                let elem = pids.number(elem);
                                let _ = writer.write_event(Event::Start(elem));
                            }
                        }
                    }
                    Place::Element | Place::Entry => {
                        let value = value.unwrap_or_default();
                        let _ = writer.write_event(Event::Start(elem));
                        clearing = 1;
                        let typed = field.place == Place::Element
                            && field
                                .key
                                .split('#')
                                .next()
                                .is_some_and(|name| TIMESTAMPS.contains(&name));
                        if !value.is_empty() && !typed {
                            let _ = writer.write_event(Event::Text(BytesText::new(value)));
                        }
                    }
                }
            }
            Event::Empty(elem) => {
                walk.enter(namespace.as_deref(), &elem);
                let elem = if walk.is_property() {
                    pids.number(elem.into_owned())
                } else {
                    elem
                };
                walk.leave();
                let _ = writer.write_event(Event::Empty(elem));
            }
            Event::End(elem) => {
                walk.leave();
                clearing = clearing.saturating_sub(1);
                let _ = writer.write_event(Event::End(elem));
            }
            // Text, CDATA, and entity references are field text: kept outside a
            // field being cleared and dropped inside one, so no part of a
            // cleared value (a CDATA section, a `Jos&#233;`) is left behind.
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) if clearing > 0 => {}
            other => {
                let _ = writer.write_event(other);
            }
        }
    }

    Bytes::from(writer.into_inner())
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
        // The first pid of a custom property is 2.
        self.highest = self.highest.max(1) + 1;
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
/// A named reference resolves through quick-xml's unescaper, which knows the five
/// XML predefined entities (`amp`, `lt`, `gt`, `quot`, `apos`). An unknown named
/// entity (a DTD-defined one, which OOXML property parts do not declare) has no
/// value here and contributes nothing, which is the safe reading: it cannot
/// smuggle text past detection.
fn resolve_ref(r: &quick_xml::events::BytesRef<'_>) -> String {
    if let Ok(Some(ch)) = r.resolve_char_ref() {
        return ch.to_string();
    }
    let name = r.clone().into_inner();
    unescape(&format!("&{name};"))
        .map(|c| c.into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::property_fields as pairs;
    use crate::opc::xml_edit::expect_edit;

    const CORE: &[u8] = br#"<?xml version="1.0" encoding="utf-8"?>
<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties">
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
    /// namespace is no field, and an element in an unreviewed namespace is
    /// neither read nor kept, even inside a field.
    #[test]
    fn a_field_is_matched_by_its_namespace() {
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:v="urn:vendor"><v:title>Jane Roe</v:title><cp:title>John Doe</cp:title><dc:title>Plan</dc:title><dc:subject>a<v:x>Ada</v:x>b</dc:subject></cp:coreProperties>"#;
        assert_eq!(
            pairs(core.as_bytes()),
            [
                ("title".to_owned(), "Plan".to_owned()),
                ("subject".to_owned(), "ab".to_owned()),
            ]
        );
        let text = surfaced_only(core);
        assert_eq!(
            text,
            r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Plan</dc:title><dc:subject>ab</dc:subject></cp:coreProperties>"#
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

        let core = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Ada</dc:creator><dc:creator>Alan</dc:creator></coreProperties>"#;
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
        let xml = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator/></coreProperties>"#;
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
        let xml = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator><![CDATA[Ada Lovelace]]></dc:creator></coreProperties>"#;
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
        let xml = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jos&#233; Bob</dc:creator></coreProperties>"#;
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
        let xml = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>&#72;&#105;</dc:creator></coreProperties>"#;
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
        let xml = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>A &amp; B</dc:creator></coreProperties>"#;
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
