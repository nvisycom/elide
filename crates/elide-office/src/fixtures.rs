//! Test fixtures for Office documents: build OPC packages and read them back.
//!
//! Reading a part out of a rewritten package means reversing the same
//! zip container the engine writes. Downstream crates test redaction by
//! asserting on individual parts, and doing so soundly means
//! *decompressing* each part, a plaintext value can straddle the deflate
//! stream, so a substring check over the raw container bytes would give a
//! false pass. This module owns that decompression so a test never has to
//! reconstruct the packaging itself.
//!
//! Gated behind the `fixtures` feature: it is scaffolding for tests in
//! this crate and its dependents, not part of the shipped surface.

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::opc::props;

/// Pack `(part name, bytes)` pairs into an OPC package, a deflate zip.
///
/// The inverse of [`read_part`], for building a minimal fixture package inline.
/// A dependent crate's test supplies exactly the parts it needs (e.g. the three
/// parts of a one-body `.docx`) without reconstructing the packaging.
///
/// # Panics
///
/// Panics if writing the in-memory zip fails, a fixture-builder error, never
/// a runtime condition.
pub fn pack_parts(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in parts {
        zip.start_file(*name, opts).expect("start fixture part");
        zip.write_all(bytes).expect("write fixture part");
    }
    zip.finish().expect("finish fixture package").into_inner()
}

/// The content type of a relationships part.
const RELATIONSHIPS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";

/// Pack `(part name, content type, bytes)` triples into an OPC package with
/// its content-types manifest: each part typed by an `Override` naming it, a
/// relationships part (`.rels`) by the manifest's `Default` for its extension
/// when its content type is empty.
///
/// # Panics
///
/// As [`pack_parts`].
pub fn pack_typed(parts: &[(&str, &str, &[u8])]) -> Vec<u8> {
    let mut manifest = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="{RELATIONSHIPS_TYPE}"/>"#
    );
    for (name, content_type, _) in parts {
        if !content_type.is_empty() {
            manifest.push_str(&format!(
                r#"<Override PartName="/{name}" ContentType="{content_type}"/>"#
            ));
        }
    }
    manifest.push_str("</Types>");
    let mut packed: Vec<(&str, &[u8])> = vec![("[Content_Types].xml", manifest.as_bytes())];
    packed.extend(parts.iter().map(|&(name, _, bytes)| (name, bytes)));
    pack_parts(&packed)
}

/// Read one part out of an OPC package by its full name, decompressed.
///
/// The name is the full part name (e.g. `word/document.xml`). Returns `None` if
/// the package cannot be opened or the part is absent.
pub fn read_part(package: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(package)).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Read one text part out of an OPC package by its full name, as UTF-8.
///
/// # Panics
///
/// Panics if the package cannot be opened, the part is absent, or it is not
/// valid UTF-8: a test reading a part it expects to be there.
#[must_use]
pub fn read_text_part(package: &[u8], name: &str) -> String {
    let bytes = read_part(package, name).unwrap_or_else(|| panic!("part `{name}` is missing"));
    String::from_utf8(bytes).unwrap_or_else(|_| panic!("part `{name}` is not valid UTF-8"))
}

/// Every part name in the package, in archive order.
///
/// Useful to sweep the whole package for leaked values.
pub fn part_names(package: &[u8]) -> Vec<String> {
    let mut zip = match ZipArchive::new(Cursor::new(package)) {
        Ok(zip) => zip,
        Err(_) => return Vec::new(),
    };
    (0..zip.len())
        .filter_map(|i| {
            let entry = zip.by_index(i).ok()?;
            Some(entry.name().ok()?.into_owned())
        })
        .collect()
}

/// Every text-bearing part, the `.xml` and `.rels` parts, as `(name, text)`.
///
/// These parts carry all of a document's text; binary parts (fonts, images,
/// media) hold none and are skipped, so a leak scan over the result covers
/// every part that could carry text PII.
///
/// A text part that is not valid UTF-8 **panics** rather than being
/// dropped: silently omitting a part a leak scan can't read would let PII
/// in that part pass unseen, the same false-pass this module exists to
/// prevent. UTF-8 is also the only encoding the package engine itself
/// accepts (a non-UTF-8 part fails extraction as `NotUtf8`), so an
/// undecodable part here is a malformed fixture, not an encoding the
/// product supports.
pub fn text_parts(package: &[u8]) -> Vec<(String, String)> {
    let mut zip = ZipArchive::new(Cursor::new(package)).expect("package is a valid zip container");
    (0..zip.len())
        .filter_map(|i| {
            let mut entry = zip.by_index(i).expect("zip entry index in range");
            let name = entry
                .name()
                .expect("zip entry name is valid UTF-8")
                .into_owned();
            if !(name.ends_with(".xml") || name.ends_with(".rels")) {
                return None;
            }
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .unwrap_or_else(|e| panic!("read part `{name}`: {e}"));
            let text = String::from_utf8(bytes)
                .unwrap_or_else(|_| panic!("text part `{name}` is not valid UTF-8"));
            Some((name, text))
        })
        .collect()
}

/// The privacy-relevant fields of a document-property part, with their values.
///
/// The part is `docProps/core.xml`, `app.xml`, or `custom.xml`; each field
/// comes back as a `(key, value)` pair with a non-empty value, keyed as
/// the property reader keys it: an element's local name (`creator`, a repeat
/// `creator#2`), a list entry by position (`TitlesOfParts/0`), or a custom
/// property by position (`property/0`, its name `property/0/name`).
#[must_use]
pub fn property_fields(xml: &[u8]) -> Vec<(String, String)> {
    props::fields(xml)
        .into_iter()
        .map(|field| (field.key, field.value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_parts_round_trips_through_read_part() {
        // A packed part reads back byte-for-byte; an absent part is `None`.
        let package = pack_parts(&[
            ("[Content_Types].xml", b"<Types/>"),
            ("word/document.xml", b"<w:document/>"),
        ]);
        assert_eq!(
            read_part(&package, "word/document.xml").as_deref(),
            Some(&b"<w:document/>"[..]),
        );
        assert!(read_part(&package, "word/missing.xml").is_none());
        // Both packed parts are enumerated.
        let names = part_names(&package);
        assert!(names.contains(&"[Content_Types].xml".to_owned()));
        assert!(names.contains(&"word/document.xml".to_owned()));
    }

    #[test]
    fn pack_typed_writes_the_manifest() {
        let package = pack_typed(&[
            ("_rels/.rels", "", b"<Relationships/>"),
            (
                "word/document.xml",
                "application/main+xml",
                b"<w:document/>",
            ),
        ]);
        let manifest = read_text_part(&package, "[Content_Types].xml");
        assert!(
            manifest.contains(r#"<Default Extension="rels""#),
            "{manifest}"
        );
        assert!(
            manifest.contains(
                r#"<Override PartName="/word/document.xml" ContentType="application/main+xml"/>"#
            ),
            "{manifest}"
        );
        assert!(!manifest.contains("/_rels/.rels"), "{manifest}");
        assert_eq!(
            read_part(&package, "word/document.xml").as_deref(),
            Some(&b"<w:document/>"[..])
        );
    }
}
