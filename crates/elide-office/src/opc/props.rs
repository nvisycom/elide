//! Reading, labelling, and clearing an OOXML document-property part
//! (`docProps/core.xml`, `docProps/app.xml`, `docProps/custom.xml`).
//!
//! These parts hold named fields that carry personal data: the author and last
//! editor, the timestamps, the company and manager, free text (the title,
//! description, keywords, template path), the names of the document's parts,
//! and custom properties. This says what each field is, reads the fields out of
//! a part's bytes, edits them back, and strips the rest when metadata is
//! treated as content, matching by element **local name** so it works whether
//! the property uses the `cp:`/`dc:`/`ap:` prefix or the default namespace, and
//! tolerating a leading byte-order mark.

use std::collections::HashMap;

use bytes::Bytes;
use elide_core::entity::{LabelRef, builtins};
use elide_core::redaction::HiddenContent;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::writer::Writer;
use quick_xml::{Reader, XmlVersion};

use crate::opc::xml_edit::{Action, Attribute, Name, Rules};

/// The OOXML namespaces whose elements are property fields: core properties,
/// the Dublin Core and DC terms elements core properties use, extended (app)
/// properties, and custom properties.
const PROPERTY_NAMESPACES: [&str; 5] = [
    "http://schemas.openxmlformats.org/package/2006/metadata/core-properties",
    "http://purl.org/dc/elements/1.1/",
    "http://purl.org/dc/terms/",
    "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties",
    "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties",
];

/// The root elements of the property parts, which hold the fields rather than
/// being one.
const PROPERTY_ROOTS: [&str; 2] = ["coreProperties", "Properties"];

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

/// The custom-properties namespace.
const CUSTOM_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties";

/// Where a field's value sits in its part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// A core or extended property element, its value its text.
    Element,
    /// A text entry of a list, keyed `<list>/<index>` (`TitlesOfParts/2`).
    Entry,
    /// A custom property, keyed by its name, its value in one typed child.
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
    /// The field's key: an element's local name, `<list>/<index>`, or a custom
    /// property's name.
    pub key: String,
    /// The field's value, trimmed.
    pub value: String,
    /// What the field is.
    pub label: LabelRef,
}

/// Element rules that strip every property no [`fields`] surfaces from a
/// property part, keeping the part's root and its surfaced fields. Applied
/// when metadata is treated as content, so what stays is exactly what
/// detection sees.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UnsurfacedProperties;

impl Rules for UnsurfacedProperties {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Properties)
    }

    fn element(&self, name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
        let property = PROPERTY_NAMESPACES
            .iter()
            .any(|namespace| name.in_namespace(namespace));
        let surfaced = ELEMENT_FIELDS.contains(&name.local)
            || LISTS.contains(&name.local)
            || (name.local == CUSTOM_PROPERTY && name.in_namespace(CUSTOM_NAMESPACE));
        if property && !PROPERTY_ROOTS.contains(&name.local) && !surfaced {
            Action::Remove
        } else {
            Action::Keep
        }
    }

    fn removes_attribute(&self, _element: Name<'_>, _attribute: &Attribute<'_>) -> bool {
        false
    }
}

/// A field an element holds: its key in the part, where its value sits, and
/// what it is.
struct Entered {
    key: String,
    place: Place,
    label: LabelRef,
}

/// The open elements of a property part, which say what field an element is.
#[derive(Debug, Default)]
struct Walk {
    /// The local names of the open elements, the root first.
    open: Vec<String>,
    /// How many text entries of the current list have been entered.
    entries: usize,
    /// How many fields each key has named so far.
    seen: HashMap<String, usize>,
}

impl Walk {
    /// Enter `elem`, returning the field it holds, if any.
    ///
    /// A key is unique in its part: a key a field has already named (a second
    /// custom property of one name, a repeated element) gets `#<n>` after it,
    /// so each field is read and rewritten on its own.
    fn enter(&mut self, elem: &BytesStart<'_>) -> Option<Entered> {
        let (name, place) = self.open_element(elem)?;
        let label = place.label(&name);
        let count = self.seen.entry(name.clone()).or_default();
        *count += 1;
        let key = match *count {
            1 => name,
            n => format!("{name}#{n}"),
        };
        Some(Entered { key, place, label })
    }

    /// Push `elem` onto the open elements, returning the field it holds by its
    /// schema name, if any.
    fn open_element(&mut self, elem: &BytesStart<'_>) -> Option<(String, Place)> {
        let local = local_name(elem);
        let field = match self.open.len() {
            1 if ELEMENT_FIELDS.contains(&local.as_str()) => Some((local.clone(), Place::Element)),
            1 if local == CUSTOM_PROPERTY => elem
                .try_get_attribute("name")
                .ok()
                .flatten()
                .and_then(|name| name.normalized_value(XmlVersion::Implicit1_0).ok())
                .map(|name| (name.into_owned(), Place::Custom)),
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
                Some((
                    format!("{}/{}", self.open[1], self.entries - 1),
                    Place::Entry,
                ))
            }
            _ => None,
        };
        self.open.push(local);
        field
    }

    /// Leave the innermost open element.
    fn leave(&mut self) {
        self.open.pop();
    }

    /// How many elements are open.
    fn depth(&self) -> usize {
        self.open.len()
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
    let Some(text) = strip_bom(xml) else {
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
                    && TEXT_TYPES.contains(&local_name(&elem).as_str())
                    && !open.value.is_empty()
                {
                    open.value.push('\n');
                }
                if reading.is_none() {
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
    let had_bom = xml.starts_with(BOM);
    // Not valid UTF-8: fail closed by preserving the original bytes rather than
    // emitting a blank part. A property part is always UTF-8 XML in practice.
    let Some(text) = strip_bom(xml) else {
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
    let mut writer = Writer::new(Vec::new());
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
                Event::Start(elem) if TEXT_TYPES.contains(&local_name(&elem).as_str()) => {
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
                let edited = field.and_then(|field| Some((value_of(&field.key)?, field)));
                let Some((value, Entered { key, place, .. })) = edited else {
                    let _ = writer.write_event(Event::Start(elem));
                    continue;
                };
                match place {
                    Place::Custom if value.is_empty() => dropping = 1,
                    Place::Custom => held = Some((elem.into_owned(), value)),
                    Place::Element | Place::Entry => {
                        let _ = writer.write_event(Event::Start(elem));
                        clearing = 1;
                        let typed = place == Place::Element && TIMESTAMPS.contains(&key.as_str());
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

    let mut out = writer.into_inner();
    if had_bom {
        let mut with_bom = Vec::with_capacity(out.len() + BOM.len());
        with_bom.extend_from_slice(BOM);
        with_bom.append(&mut out);
        out = with_bom;
    }
    Bytes::from(out)
}

/// The UTF-8 byte-order mark some Office parts carry.
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// `bytes` as `&str`, skipping a leading BOM, or `None` when the bytes are not
/// valid UTF-8 so a caller can fail closed rather than silently emit nothing.
fn strip_bom(bytes: &[u8]) -> Option<&str> {
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    std::str::from_utf8(bytes).ok()
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
    use crate::opc::xml_edit::edit;

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
        let edited = edit(core, &UnsurfacedProperties).unwrap().unwrap();
        assert_eq!(edited.count, 2);
        for kept in ["Jane Roe", "Plan", "acquisition"] {
            assert!(edited.text.contains(kept), "{kept} was stripped");
        }
        assert!(!edited.text.contains("revision") && !edited.text.contains("Draft"));

        let app = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Template>C:\Users\jroe\Normal.dotm</Template><Company>Acme</Company><HyperlinkBase>https://intranet</HyperlinkBase><HeadingPairs><vt:vector size="2" baseType="variant"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>1</vt:i4></vt:variant></vt:vector></HeadingPairs><TitlesOfParts><vt:vector size="1" baseType="lpstr"><vt:lpstr>Secret sheet</vt:lpstr></vt:vector></TitlesOfParts><Application>Microsoft Excel</Application></Properties>"#;
        let edited = edit(app, &UnsurfacedProperties).unwrap().unwrap();
        assert_eq!(edited.count, 1);
        for kept in ["jroe", "Acme", "intranet", "Worksheets", "Secret sheet"] {
            assert!(edited.text.contains(kept), "{kept} was stripped");
        }
        assert!(!edited.text.contains("Excel"));

        let custom = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property></Properties>"#;
        assert!(edit(custom, &UnsurfacedProperties).unwrap().is_none());
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
    fn a_custom_property_is_a_field_by_its_name() {
        let got = fields(CUSTOM);
        assert_eq!(
            pairs(CUSTOM),
            [
                ("Client".to_owned(), "Jane Roe".to_owned()),
                ("creator".to_owned(), "42".to_owned()),
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
            [(
                "Reviewers".to_owned(),
                "Jane Roe\ncarol@example.com".to_owned()
            )]
        );
        // A list cannot hold the edited text, so the property goes whole.
        let out = rewrite(custom, &[("Reviewers", "Jane Roe\n[EMAIL]")]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            !text.contains("Reviewers") && !text.contains("carol"),
            "{text}"
        );
    }

    #[test]
    fn a_repeated_name_is_its_own_field() {
        let custom = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="Client"><vt:lpwstr>ACME</vt:lpwstr></property></Properties>"#;
        assert_eq!(
            pairs(custom),
            [
                ("Client".to_owned(), "Jane Roe".to_owned()),
                ("Client#2".to_owned(), "ACME".to_owned()),
            ]
        );
        let out = rewrite(custom, &[("Client", "[NAME]")]);
        assert_eq!(
            pairs(&out),
            [
                ("Client".to_owned(), "[NAME]".to_owned()),
                ("Client#2".to_owned(), "ACME".to_owned()),
            ]
        );

        let core = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Ada</dc:creator><dc:creator>Alan</dc:creator></coreProperties>"#;
        let got = fields(core);
        assert_eq!(got[1].key, "creator#2");
        assert_eq!(got[1].label, LabelRef::from(&*builtins::PERSON_NAME));
    }

    #[test]
    fn a_custom_property_is_overwritten_in_its_text_or_dropped() {
        let out = rewrite(CUSTOM, &[("Client", "[NAME]"), ("creator", "[NAME]")]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            text.contains(r#"name="Client"><vt:lpwstr>[NAME]</vt:lpwstr></property>"#),
            "{text}"
        );
        // A number cannot hold the text, so the property goes whole.
        assert!(!text.contains("creator") && !text.contains("42"), "{text}");

        let out = rewrite(CUSTOM, &[("Client", "")]);
        assert_eq!(pairs(&out), [("creator".to_owned(), "42".to_owned())]);
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
