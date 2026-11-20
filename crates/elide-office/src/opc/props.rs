//! Reading, labelling, and clearing an OOXML document-property part
//! (`docProps/core.xml`, `docProps/app.xml`, `docProps/custom.xml`).
//!
//! These parts hold named fields that carry personal data: the author and last
//! editor, the timestamps, the company and manager, free text (the title,
//! description, keywords, template path), the names of the document's parts,
//! and custom properties. This says what each field is, reads the fields out of
//! a part's bytes, and edits them back, matching by element **local name** so
//! it works whether the property uses the `cp:`/`dc:`/`ap:` prefix or the
//! default namespace, and tolerating a leading byte-order mark. When metadata
//! is treated as content, [`UnsurfacedProperties`] strips everything else by
//! the same walk, so the part keeps exactly what [`fields`] reads.

use std::collections::HashMap;
use std::ops::Range;

use bytes::Bytes;
use elide_core::Result;
use elide_core::entity::{LabelRef, builtins};
use elide_core::redaction::HiddenContent;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::writer::Writer;
use quick_xml::{Reader, XmlVersion};

use crate::opc::malformed_xml;
use crate::opc::xml_edit::{Edit, Edited, SpannedReader, attribute_source, bom_len, splice};

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
/// It walks the part as [`fields`] does, by local name in any namespace, and
/// keeps the root, each surfaced field with everything inside it, and the
/// list containers, vectors, and variants on the path to a kept list entry.
/// Every other element goes with its subtree, as do text, CDATA, and entity
/// references outside a field, an entity reference no field resolves, and
/// every processing instruction; whitespace between elements stays. A vector
/// that loses entries has its `size` set to the entries it keeps.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UnsurfacedProperties;

impl Edit for UnsurfacedProperties {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Properties)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        let mut reader = SpannedReader::new(raw);
        let mut walk = Walk::default();
        let mut cuts = Cuts::default();
        // The kept elements that are not fields: the root, and the list
        // containers, vectors, and variants, kept only when an entry is.
        let mut frames: Vec<Frame> = Vec::new();
        // The depth of the surfaced field being read, everything inside kept.
        let mut reading: Option<usize> = None;
        // The depth and start of the element being removed.
        let mut removing: Option<(usize, usize)> = None;

        loop {
            let (event, span) = reader.read_event().map_err(malformed_xml)?;
            match event {
                Event::Eof => break,
                Event::DocType(_) => return Err(malformed_xml("a document type declaration")),
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let is_start = matches!(event, Event::Start(_));
                    let depth = walk.depth();
                    let field = walk.enter(e);
                    if !is_start {
                        walk.leave();
                    }
                    if removing.is_some() || reading.is_some() {
                        continue;
                    }
                    if depth == 0 {
                        if is_start {
                            frames.push(Frame::new(depth, span.start, &cuts, None));
                        }
                    } else if let Some(field) = field {
                        // A custom property's name is read only when it holds
                        // a value, so an empty one is never surfaced.
                        if is_start || field.place != Place::Custom {
                            keep_child(&mut frames);
                            if is_start {
                                reading = Some(depth);
                            }
                        } else {
                            cuts.remove(span);
                        }
                    } else if is_start && walk.is_list_path() {
                        frames.push(Frame::new(depth, span.start, &cuts, size(raw, e)?));
                    } else if is_start {
                        removing = Some((depth, span.start));
                    } else {
                        cuts.remove(span);
                    }
                }
                Event::End(_) => {
                    walk.leave();
                    let depth = walk.depth();
                    if let Some((removed, start)) = removing {
                        if removed == depth {
                            cuts.remove(start..span.end);
                            removing = None;
                        }
                    } else if reading == Some(depth) {
                        reading = None;
                    } else if reading.is_none()
                        && let Some(frame) = frames.pop_if(|frame| frame.depth == depth)
                    {
                        if frame.children == 0 && depth > 0 {
                            cuts.truncate(frame.cuts);
                            cuts.remove(frame.start..span.end);
                        } else {
                            if let Some((value, written)) = frame.size
                                && written != frame.children.to_string()
                            {
                                cuts.replace(value, frame.children.to_string());
                            }
                            keep_child(&mut frames);
                        }
                    }
                }
                _ if removing.is_some() => {}
                Event::Text(ref text) if reading.is_none() && !is_blank(text) => cuts.remove(span),
                Event::CData(_) if reading.is_none() => cuts.remove(span),
                Event::GeneralRef(ref r) if reading.is_none() || resolve_ref(r).is_empty() => {
                    cuts.remove(span);
                }
                Event::PI(_) => cuts.remove(span),
                _ => {}
            }
        }
        Ok(cuts.into_edited(raw))
    }
}

/// The cuts of an [`UnsurfacedProperties`] edit, in document order: each
/// range removed (counted) or a vector's `size` rewritten (not counted).
#[derive(Default)]
struct Cuts(Vec<(Range<usize>, String, bool)>);

impl Cuts {
    /// Remove `range`.
    fn remove(&mut self, range: Range<usize>) {
        self.0.push((range, String::new(), true));
    }

    /// Write `with` in place of `range`.
    fn replace(&mut self, range: Range<usize>, with: String) {
        self.0.push((range, with, false));
    }

    /// Forget every cut after the first `len`, for an element removed whole.
    fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    /// `raw` with these cuts, or `None` when there are none.
    fn into_edited(self, raw: &str) -> Option<Edited> {
        if self.0.is_empty() {
            return None;
        }
        let count = self.0.iter().filter(|(_, _, counted)| *counted).count();
        let cuts = self
            .0
            .into_iter()
            .map(|(range, with, _)| (range, with))
            .collect();
        Some(Edited {
            text: splice(raw, cuts),
            count,
        })
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
    fn new(depth: usize, start: usize, cuts: &Cuts, size: Option<(Range<usize>, String)>) -> Self {
        Self {
            depth,
            start,
            cuts: cuts.0.len(),
            size,
            children: 0,
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
}

impl Walk {
    /// Enter `elem`, returning the field it holds, if any.
    ///
    /// Every key is unique in its part. An element field is keyed by its name,
    /// a repeated one with `#<n>` after it (no element name holds a `#`). A
    /// list entry is keyed `<list>/<n>`, and a custom property by its position,
    /// `property/<n>`, since its name is free text that could look like any
    /// key; the name is a field of its own, `property/<n>/name`.
    fn enter(&mut self, elem: &BytesStart<'_>) -> Option<Entered> {
        let local = local_name(elem);
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
                .and_then(|name| name.normalized_value(XmlVersion::Implicit1_0).ok())
                .map(|name| {
                    self.properties += 1;
                    Entered {
                        key: format!("{CUSTOM_PROPERTY}/{}", self.properties - 1),
                        place: Place::Custom,
                        label: Place::Custom.label(&local),
                        name: Some(name.into_owned()),
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
    }

    /// How many elements are open.
    fn depth(&self) -> usize {
        self.open.len()
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
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut fields = Vec::new();
    let mut walk = Walk::default();
    let mut reading: Option<Reading> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(elem)) => {
                let field = walk.enter(&elem);
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
            Ok(Event::Empty(elem)) => {
                walk.enter(&elem);
                walk.leave();
            }
            Ok(Event::Text(t)) => {
                if let Some(open) = &mut reading {
                    open.value
                        .push_str(&unescape(t.as_ref()).unwrap_or_default());
                }
            }
            // A field value written as CDATA carries its text verbatim (no entity
            // escaping), so it is appended as-is; without this a `<dc:creator>`
            // stored as CDATA would read as empty and never be surfaced.
            Ok(Event::CData(t)) => {
                if let Some(open) = &mut reading {
                    open.value.push_str(&t);
                }
            }
            // quick-xml emits an entity reference (`&#233;`, `&amp;`) as its own
            // event, not as part of the surrounding text. Resolve it into the
            // value; without this a name like `Jos&#233;`, or a wholly
            // entity-encoded value, would be read short and slip past detection.
            Ok(Event::GeneralRef(r)) => {
                if let Some(open) = &mut reading {
                    open.value.push_str(&resolve_ref(&r));
                }
            }
            Ok(Event::End(_)) => {
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
            Ok(Event::Eof) => break,
            Err(_) => break,
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
/// whole, since an empty or mistyped value is not valid. A key that names no
/// field is a no-op; the rest of the document is preserved in structure.
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
    let mut reader = Reader::from_str(text);
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

    loop {
        let event = match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(event) => event,
            Err(_) => return Bytes::copy_from_slice(xml),
        };
        if dropping > 0 {
            match event {
                Event::Start(elem) => {
                    walk.enter(&elem);
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
                    walk.enter(&elem);
                    let _ = writer.write_event(Event::Start(property));
                    let _ = writer.write_event(Event::Start(elem));
                    let _ = writer.write_event(Event::Text(BytesText::new(value)));
                    clearing = 2;
                }
                Event::Start(elem) => {
                    walk.enter(&elem);
                    dropping = 2;
                }
                Event::Empty(_) => dropping = 1,
                Event::End(_) => walk.leave(),
                // Text before the typed value is whitespace; keep waiting.
                _ => held = Some((property, value)),
            }
            continue;
        }
        match event {
            Event::Start(elem) => {
                let field = walk.enter(&elem);
                if clearing > 0 {
                    clearing += 1;
                    let _ = writer.write_event(Event::Start(elem));
                    continue;
                }
                let Some(field) = field else {
                    let _ = writer.write_event(Event::Start(elem));
                    continue;
                };
                let value = value_of(&field.key);
                let name = (field.place == Place::Custom)
                    .then(|| value_of(&field.name_key()))
                    .flatten();
                if value.is_none() && name.is_none() {
                    let _ = writer.write_event(Event::Start(elem));
                    continue;
                }
                match field.place {
                    // A custom property needs a name and a value: a blank one
                    // drops it whole.
                    Place::Custom if value == Some("") || name == Some("") => dropping = 1,
                    Place::Custom => {
                        let elem = match name {
                            Some(name) => renamed(&elem, name),
                            None => elem.into_owned(),
                        };
                        match value {
                            Some(value) => held = Some((elem, value)),
                            None => {
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
                walk.enter(&elem);
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
    let mut renamed = BytesStart::new(elem.name().as_ref().to_owned());
    for attribute in elem.attributes().flatten() {
        if attribute.key.as_ref() == "name" {
            renamed.push_attribute(("name", name));
        } else {
            renamed.push_attribute(attribute);
        }
    }
    renamed
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

/// The local name of an element, dropping any namespace prefix (`dc:creator` →
/// `creator`), via quick-xml's own prefix handling.
fn local_name(elem: &BytesStart<'_>) -> String {
    elem.local_name().as_ref().to_owned()
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
    use crate::opc::xml_edit::expect_edit;

    /// The `(key, value)` of each field `xml` surfaces.
    fn pairs(xml: &[u8]) -> Vec<(String, String)> {
        fields(xml)
            .into_iter()
            .map(|field| (field.key, field.value))
            .collect()
    }

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
        // The application, and the heading's count, which no field reads.
        assert_eq!(edited.count, 2);
        for kept in ["jroe", "Acme", "intranet", "Worksheets", "Secret sheet"] {
            assert!(edited.text.contains(kept), "{kept} was stripped");
        }
        assert!(!edited.text.contains("Excel") && !edited.text.contains("vt:i4"));
        assert!(
            edited
                .text
                .contains(r#"<vt:vector size="1" baseType="variant">"#)
        );

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
    fn unread(xml: &str) -> Vec<String> {
        let read: Vec<String> = fields(xml.as_bytes())
            .into_iter()
            .map(|field| field.value)
            .collect();
        let mut reader = Reader::from_str(xml);
        let mut unread = Vec::new();
        loop {
            let text = match reader.read_event().unwrap() {
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
            text.contains("Worksheets") && text.contains(r#"size="1""#),
            "{text}"
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
