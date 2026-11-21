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
//! is treated as content, [`UnsurfacedProperties`](strip::UnsurfacedProperties)
//! strips everything else by the same walk, so the part keeps exactly what
//! [`fields`] reads.
//!
//! The field model lives here: [`Field`], [`fields`], and [`rewrite`]. The
//! [`walk`] that says what field an element holds is shared with the [`strip`]
//! edit, and [`vocabulary`] names the namespaces a property part may use.

pub(super) mod strip;
#[cfg(test)]
mod tests;
pub(super) mod vocabulary;
mod walk;

use std::{io, mem};

use bytes::Bytes;
use elide_core::entity::LabelRef;
use quick_xml::NsReader;
use quick_xml::escape::{resolve_predefined_entity, unescape};
use quick_xml::events::{BytesRef, BytesStart, BytesText, Event};
use quick_xml::writer::Writer;

use self::walk::{Entered, Place, Walk};
use crate::opc::attribute_value;
use crate::opc::xml::edit::{bom_len, bound};

/// The property parts detection reads, at the paths it reads them from. A
/// property part anywhere else is never surfaced, so it is stripped as
/// hidden content whatever the policy (see [`PartKind`](crate::opc::PartKind)).
pub(crate) const SURFACED_PARTS: &[&str] = &[
    "docProps/core.xml",
    "docProps/app.xml",
    "docProps/custom.xml",
];

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

/// The element fields that hold a typed W3CDTF timestamp, which is removed
/// rather than overwritten with arbitrary text or left empty.
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
/// value becomes its only text, and a blank one leaves it empty; a field
/// holding a typed timestamp given any value is removed whole. A custom property takes its new
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
    /// Drop a custom property or a redacted timestamp, this many elements
    /// open in it, itself included: nothing of it is written.
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
            // A timestamp cannot hold a redacted value, and an empty one is
            // not a W3CDTF date: it goes whole. Every core property is
            // optional.
            Place::Element | Place::Entry if field.timestamp => {
                self.mode = Mode::Dropping(1);
            }
            Place::Element | Place::Entry => {
                let value = value.unwrap_or_default();
                self.write(Event::Start(elem))?;
                self.mode = Mode::Clearing(1);
                if !value.is_empty() {
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
