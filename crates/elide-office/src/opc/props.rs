//! Reading, labelling, and clearing an OOXML document-property part
//! (`docProps/core.xml`, `docProps/app.xml`, `docProps/custom.xml`).
//!
//! These parts hold named fields that carry personal data (the author and last
//! editor, the timestamps, the company and manager, the title and description)
//! as XML elements. This says what each field is, reads the labelled ones out
//! of a part's bytes, clears them back out, and strips the rest when metadata
//! is redacted, matching by element **local name** so it works whether the
//! property uses the `cp:`/`dc:`/`ap:` prefix or the default namespace, and
//! tolerating a leading byte-order mark.

use bytes::Bytes;
use elide_core::entity::{LabelRef, builtins};
use elide_core::redaction::HiddenContent;
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesText, Event};
use quick_xml::writer::Writer;

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

/// What a property field (by XML local name) is, or `None` when the schema does
/// not fix it.
///
/// One table serves `core.xml` and `app.xml`: matching by local name means it
/// holds whatever prefix a part binds. A field this table labels is surfaced
/// for detection; when metadata is treated as content every other field is
/// stripped, since nothing could detect what it holds.
#[must_use]
pub fn label(local_name: &str) -> Option<LabelRef> {
    let label = match local_name {
        "creator" | "lastModifiedBy" | "Manager" => &builtins::PERSON_NAME,
        "created" | "modified" => &builtins::DATE_TIME,
        "Company" => &builtins::ORGANIZATION_NAME,
        "title" | "subject" | "description" => &builtins::COMMUNICATIONS_CONTENT,
        _ => return None,
    };
    Some(LabelRef::from(&**label))
}

/// Whether `local_name` is a property field surfaced for detection: one the
/// schema [labels](label).
#[must_use]
pub fn is_sensitive(local_name: &str) -> bool {
    label(local_name).is_some()
}

/// Element rules that strip every property field no [`label`] covers from a
/// property part, keeping the part's root and its labelled fields. Applied
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
        if property && !PROPERTY_ROOTS.contains(&name.local) && !is_sensitive(name.local) {
            Action::Remove
        } else {
            Action::Keep
        }
    }

    fn removes_attribute(&self, _element: Name<'_>, _attribute: &Attribute<'_>) -> bool {
        false
    }
}

/// The privacy-relevant fields a property part carries, as `(local_name, value)`
/// pairs. Only fields with a non-empty value are returned, so an already-empty
/// `<dc:creator/>` is not surfaced as a redaction subject.
#[must_use]
pub fn fields(xml: &[u8]) -> Vec<(String, String)> {
    // Not valid UTF-8: no fields are readable (a property part is always UTF-8).
    let Some(text) = strip_bom(xml) else {
        return Vec::new();
    };
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut fields = Vec::new();
    let mut current: Option<String> = None;
    let mut value = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(elem)) => {
                let name = local_name(&elem);
                if is_sensitive(&name) {
                    current = Some(name);
                    value.clear();
                }
            }
            Ok(Event::Text(t)) if current.is_some() => {
                value.push_str(&unescape(t.as_ref()).unwrap_or_default());
            }
            // A field value written as CDATA carries its text verbatim (no entity
            // escaping), so it is appended as-is; without this a `<dc:creator>`
            // stored as CDATA would read as empty and never be surfaced.
            Ok(Event::CData(t)) if current.is_some() => {
                value.push_str(&t);
            }
            // quick-xml emits an entity reference (`&#233;`, `&amp;`) as its own
            // event, not as part of the surrounding text. Resolve it into the
            // value; without this a name like `Jos&#233;`, or a wholly
            // entity-encoded value, would be read short and slip past detection.
            Ok(Event::GeneralRef(r)) if current.is_some() => {
                value.push_str(&resolve_ref(&r));
            }
            Ok(Event::End(_)) => {
                if let Some(name) = current.take()
                    && !value.trim().is_empty()
                {
                    fields.push((name, value.trim().to_owned()));
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    fields
}

/// Whether the field `local_name` holds free text, so it can be overwritten. A
/// timestamp holds a typed W3CDTF value, so it is cleared instead.
#[must_use]
pub fn is_writable(local_name: &str) -> bool {
    is_sensitive(local_name) && !matches!(local_name, "created" | "modified")
}

/// Clear the text of every field named in `removed` and overwrite the text of
/// every field named in `replaced` with its new value (each a local name),
/// returning the edited XML. A field that cannot hold free text is cleared
/// instead of overwritten. A named field that is not present, or not
/// sensitive, is a no-op; the rest of the document is preserved byte-for-byte
/// in structure.
///
/// Fails closed: on non-UTF-8 input or a mid-stream parse error, the original
/// bytes are returned unchanged rather than a blank or truncated part, since a
/// metadata part that cannot be rewritten is better left intact than corrupted.
#[must_use]
pub fn rewrite(xml: &[u8], removed: &[&str], replaced: &[(&str, &str)]) -> Bytes {
    let had_bom = xml.starts_with(BOM);
    // Not valid UTF-8: fail closed by preserving the original bytes rather than
    // emitting a blank part. A property part is always UTF-8 XML in practice.
    let Some(text) = strip_bom(xml) else {
        return Bytes::copy_from_slice(xml);
    };
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    // Depth inside a to-be-cleared element, so nested text is dropped and the
    // element re-emits empty.
    let mut clearing = 0u32;

    loop {
        match reader.read_event() {
            Ok(Event::Start(elem)) => {
                let name = local_name(&elem);
                let replacement = replaced
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| *value);
                let clear = (removed.contains(&name.as_str()) || replacement.is_some())
                    && is_sensitive(&name);
                let _ = writer.write_event(Event::Start(elem.clone()));
                // Enter (or stay in) the clearing region: a to-be-cleared element,
                // or any element nested inside one, drops its text.
                if clear || clearing > 0 {
                    clearing += 1;
                }
                // An overwritten field's new value is its only text.
                if clearing == 1
                    && let Some(value) = replacement.filter(|_| is_writable(&name))
                {
                    let _ = writer.write_event(Event::Text(BytesText::new(value)));
                }
            }
            Ok(Event::End(elem)) => {
                clearing = clearing.saturating_sub(1);
                let _ = writer.write_event(Event::End(elem));
            }
            Ok(Event::Text(t)) => {
                if clearing == 0 {
                    let _ = writer.write_event(Event::Text(t));
                } else {
                    // Drop the text, emitting nothing between the tags.
                    let _ = writer.write_event(Event::Text(BytesText::new("")));
                }
            }
            // A CDATA-wrapped value is field text too: keep it outside a clearing
            // region, drop it inside one. Without this arm the catch-all below
            // would pass CDATA through and leave a cleared field's value behind.
            Ok(Event::CData(t)) => {
                if clearing == 0 {
                    let _ = writer.write_event(Event::CData(t));
                } else {
                    let _ = writer.write_event(Event::Text(BytesText::new("")));
                }
            }
            // An entity reference is field text quick-xml surfaces separately:
            // keep it verbatim outside a clearing region, drop it inside one.
            // Without this arm the catch-all below would forward the reference
            // and leave part of a cleared field's value (e.g. `Jos&#233;`) behind.
            Ok(Event::GeneralRef(r)) => {
                if clearing == 0 {
                    let _ = writer.write_event(Event::GeneralRef(r));
                } else {
                    let _ = writer.write_event(Event::Text(BytesText::new("")));
                }
            }
            Ok(Event::Eof) => break,
            Ok(other) => {
                let _ = writer.write_event(other);
            }
            Err(_) => return Bytes::copy_from_slice(xml),
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
fn local_name(elem: &quick_xml::events::BytesStart<'_>) -> String {
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

    const CORE: &[u8] = br#"<?xml version="1.0" encoding="utf-8"?>
<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties">
  <dc:creator>Ada Lovelace</dc:creator>
  <lastModifiedBy>Alan Turing</lastModifiedBy>
  <dc:title></dc:title>
  <revision>7</revision>
</coreProperties>"#;

    #[test]
    fn reads_populated_sensitive_fields_only() {
        let got = fields(CORE);
        assert!(got.contains(&("creator".to_owned(), "Ada Lovelace".to_owned())));
        assert!(got.contains(&("lastModifiedBy".to_owned(), "Alan Turing".to_owned())));
        // Empty title and non-sensitive revision are not surfaced.
        assert!(!got.iter().any(|(k, _)| k == "title" || k == "revision"));
    }

    #[test]
    fn redacting_keeps_only_the_labelled_fields() {
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jane Roe</dc:creator><dc:title>Plan</dc:title><cp:keywords>acquisition</cp:keywords><cp:revision>7</cp:revision></cp:coreProperties>"#;
        let edited = edit(core, &UnsurfacedProperties).unwrap().unwrap();
        assert_eq!(edited.count, 2);
        assert!(edited.text.contains("Jane Roe") && edited.text.contains("Plan"));
        assert!(!edited.text.contains("acquisition") && !edited.text.contains("revision"));

        let app = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Template>C:\Users\jroe\Normal.dotm</Template><Company>Acme</Company><HyperlinkBase>https://intranet</HyperlinkBase><TitlesOfParts><vt:vector size="1" baseType="lpstr"><vt:lpstr>Secret sheet</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#;
        let edited = edit(app, &UnsurfacedProperties).unwrap().unwrap();
        assert_eq!(edited.count, 3);
        assert!(edited.text.contains("Acme"));
        for leaked in ["jroe", "intranet", "Secret sheet"] {
            assert!(!edited.text.contains(leaked), "{leaked} survived");
        }

        let custom = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Client"><vt:lpwstr>Jane Roe</vt:lpwstr></property></Properties>"#;
        let edited = edit(custom, &UnsurfacedProperties).unwrap().unwrap();
        assert_eq!(edited.count, 1);
        assert!(!edited.text.contains("Jane Roe"));

        let reduced = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jane Roe</dc:creator></cp:coreProperties>"#;
        assert!(edit(reduced, &UnsurfacedProperties).unwrap().is_none());
    }

    #[test]
    fn rewrite_overwrites_a_text_field_and_clears_a_timestamp() {
        let xml = br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/"><dc:creator>Ada &amp; co</dc:creator><dcterms:created>2024-01-02T03:04:05Z</dcterms:created></cp:coreProperties>"#;
        let out = rewrite(xml, &[], &[("creator", "[A & B]"), ("created", "never")]);
        let after = fields(&out);
        assert!(after.contains(&("creator".to_owned(), "[A & B]".to_owned())));
        assert!(!after.iter().any(|(k, _)| k == "created"));
        assert!(!std::str::from_utf8(&out).unwrap().contains("never"));
    }

    #[test]
    fn strip_clears_named_fields_and_keeps_the_rest() {
        let out = rewrite(CORE, &["creator"], &[]);
        let after = fields(&out);
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
        let out = rewrite(&with_bom, &["creator"], &[]);
        assert!(out.starts_with(BOM), "BOM dropped");
    }

    #[test]
    fn strip_of_non_utf8_returns_the_original_bytes() {
        // A lone continuation byte is not valid UTF-8: fail closed, don't blank.
        let garbage: &[u8] = &[0x3c, 0xFF, 0x3e];
        let out = rewrite(garbage, &["creator"], &[]);
        assert_eq!(&out[..], garbage, "non-UTF-8 input was not preserved");
    }

    #[test]
    fn a_self_closing_sensitive_element_carries_no_value() {
        // `<dc:creator/>` has no text, so it is neither surfaced nor changed.
        let xml = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator/></coreProperties>"#;
        assert!(fields(xml).is_empty(), "empty self-closing field surfaced");
        let out = rewrite(xml, &["creator"], &[]);
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
            fields(xml).contains(&("creator".to_owned(), "Ada Lovelace".to_owned())),
            "CDATA field value not surfaced"
        );

        let out = rewrite(xml, &["creator"], &[]);
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
            fields(xml).contains(&("creator".to_owned(), "José Bob".to_owned())),
            "numeric-entity value not fully surfaced: {:?}",
            fields(xml)
        );

        let out = rewrite(xml, &["creator"], &[]);
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
            fields(xml).contains(&("creator".to_owned(), "Hi".to_owned())),
            "all-entity value not surfaced: {:?}",
            fields(xml)
        );
    }

    #[test]
    fn a_named_entity_reference_is_read_and_stripped() {
        // A predefined named entity (`&amp;`) inside a value: read as `&` and
        // cleared on strip.
        let xml = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>A &amp; B</dc:creator></coreProperties>"#;
        assert!(
            fields(xml).contains(&("creator".to_owned(), "A & B".to_owned())),
            "named-entity value not surfaced: {:?}",
            fields(xml)
        );

        let out = rewrite(xml, &["creator"], &[]);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            !text.contains("&amp;"),
            "named entity leaked past strip: {text}"
        );
        assert!(!text.contains(" B"), "value text leaked past strip: {text}");
    }
}
