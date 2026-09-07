//! Reading and clearing a DOCX document-property part (`docProps/core.xml`,
//! `docProps/app.xml`).
//!
//! These parts hold named fields that carry personal data — the author and last
//! editor, the create/modify timestamps, the company and manager — as XML
//! elements. This reads the privacy-relevant ones out of a part's bytes and
//! clears them back out, matching by element **local name** so it works whether
//! the property uses the `cp:`/`dc:`/`ap:` prefix or the default namespace, and
//! tolerating a leading byte-order mark.

use bytes::Bytes;
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesText, Event};
use quick_xml::writer::Writer;

/// The privacy-relevant property fields, by XML local name. A field not in this
/// set (a revision count, an app version, a language tag) is left untouched.
///
/// Spans both core and extended properties: matching by local name means one
/// list serves `core.xml` (`creator`, `lastModifiedBy`, `created`, …) and
/// `app.xml` (`Company`, `Manager`).
const SENSITIVE: &[&str] = &[
    // core.xml
    "creator",
    "lastModifiedBy",
    "title",
    "subject",
    "description",
    "keywords",
    "category",
    "created",
    "modified",
    // app.xml
    "Company",
    "Manager",
];

/// Whether `local_name` is a privacy-relevant property field.
#[must_use]
pub fn is_sensitive(local_name: &str) -> bool {
    SENSITIVE.contains(&local_name)
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

/// Clear the text of every field named in `keys` (each a local name), returning
/// the edited XML. A named field that is not present, or not sensitive, is a
/// no-op; the rest of the document is preserved byte-for-byte in structure.
///
/// Fails closed: on non-UTF-8 input or a mid-stream parse error, the original
/// bytes are returned unchanged rather than a blank or truncated part, since a
/// metadata part that cannot be rewritten is better left intact than corrupted.
#[must_use]
pub fn strip(xml: &[u8], keys: &[&str]) -> Bytes {
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
                let clear = keys.contains(&name.as_str()) && is_sensitive(&name);
                let _ = writer.write_event(Event::Start(elem.clone()));
                // Enter (or stay in) the clearing region: a to-be-cleared element,
                // or any element nested inside one, drops its text.
                if clear || clearing > 0 {
                    clearing += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn strip_clears_named_fields_and_keeps_the_rest() {
        let out = strip(CORE, &["creator"]);
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
        let out = strip(&with_bom, &["creator"]);
        assert!(out.starts_with(BOM), "BOM dropped");
    }

    #[test]
    fn strip_of_non_utf8_returns_the_original_bytes() {
        // A lone continuation byte is not valid UTF-8: fail closed, don't blank.
        let garbage: &[u8] = &[0x3c, 0xFF, 0x3e];
        let out = strip(garbage, &["creator"]);
        assert_eq!(&out[..], garbage, "non-UTF-8 input was not preserved");
    }

    #[test]
    fn a_self_closing_sensitive_element_carries_no_value() {
        // `<dc:creator/>` has no text, so it is neither surfaced nor changed.
        let xml = br#"<coreProperties xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator/></coreProperties>"#;
        assert!(fields(xml).is_empty(), "empty self-closing field surfaced");
        let out = strip(xml, &["creator"]);
        assert!(
            std::str::from_utf8(&out).unwrap().contains("creator"),
            "self-closing element should survive structurally"
        );
    }
}
