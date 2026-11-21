//! [`check`]: confirm an encoded OOXML package carries nothing the codec
//! should not ship.
//!
//! The check a document runs on its output before returning it: sanitizing
//! the output again changes nothing (so every XML part rebuilt from its schema
//! is a fixed point of the rebuild, and holds only the schema's namespaces),
//! every relationship and every element naming one resolves, the manifest and
//! the relationships parts hold only their OPC vocabulary, every property
//! part, kept but never rebuilt, speaks only reviewed vocabularies, and no
//! part the format never extracts carries text.

use std::collections::HashSet;

use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::opc::archive::RawPart;
use crate::opc::index::{Index, text};
use crate::opc::part::PartPath;
use crate::opc::references::References;
use crate::opc::sanitize::{Sanitized, Sanitizer, rebuilt, sanitize};
use crate::opc::vocabulary::UnreviewedNamespaces;
use crate::opc::{PartKind, PartRole, malformed_xml, package_xml, xml_edit};

/// Check that the package `document` carries no hidden content `policy` and
/// `sanitizer` would remove, that every reference in it resolves, that its
/// property parts use only reviewed vocabularies, and that no part the format
/// does not extract carries text.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) naming what survived or what dangles, or the
/// errors of [`sanitize`].
pub(crate) fn check(
    document: &[u8],
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<()> {
    check_sanitized(document, policy, sanitizer)?;
    let index = Index::read(document)?;
    check_relationships(&index)?;
    for part in &index.parts {
        // The manifest and the relationships parts: the sanitize above cuts
        // them to their OPC vocabulary and records every cut, so what it left
        // is all there is.
        if package_xml::vocabulary_of(&part.path).is_some() || !index.is_xml(&part.path) {
            continue;
        }
        check_xml_part(&index, part, sanitizer)?;
    }
    Ok(())
}

/// Check that sanitizing `document` again removes nothing and changes
/// nothing.
fn check_sanitized(
    document: &[u8],
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<()> {
    let Sanitized { bytes, stripped } = sanitize(document, policy, sanitizer)?;
    if let Some(entry) = stripped.iter().next() {
        let place = entry.at.as_ref().map_or("the package", |at| at.as_str());
        return Err(Error::sanitization(format!(
            "encoded package still carries hidden content: {:?} in {place}",
            entry.content
        )));
    }
    // A rewrite that removes nothing still changes the package, which is then
    // not what a sanitize writes.
    if bytes.is_some_and(|changed| changed != document) {
        return Err(Error::sanitization(
            "encoded package is not what a sanitize writes: sanitizing it again changes it",
        ));
    }
    Ok(())
}

/// Check that every relationship of the package `index` reads targets a part
/// it holds.
fn check_relationships(index: &Index) -> Result<()> {
    let present: HashSet<&PartPath> = index.parts.iter().map(|part| &part.path).collect();
    for (rels_path, rels) in &index.relationships {
        for rel in &rels.relationships {
            if let Some(target) = rel.target_part(rels.source.as_ref())
                && !present.contains(&target)
            {
                return Err(Error::sanitization(format!(
                    "encoded package has a dangling reference: relationship `{}` in \
                     `{rels_path}` targets missing part `{target}`",
                    rel.id
                )));
            }
        }
    }
    Ok(())
}

/// Check the XML `part` of the package `index`: every relationship it names
/// resolves, it carries no text if the format never extracts it, and, if it
/// is not rebuilt from its schema, it speaks only reviewed vocabularies.
fn check_xml_part(index: &Index, part: &RawPart, sanitizer: &dyn Sanitizer) -> Result<()> {
    let RawPart { path, bytes } = part;
    let raw = text(path, bytes)?;
    let ids: HashSet<String> = index
        .relationships_of(path)
        .iter()
        .map(|rel| rel.id.clone())
        .collect();
    let references = References::checking(&ids);
    xml_edit::edit(raw, &references)?;
    if let Some(id) = references.dangling() {
        return Err(Error::sanitization(format!(
            "encoded package has a dangling reference: `{path}` names missing \
             relationship `{id}`"
        )));
    }
    let kind = index.kind(path);
    // A structure part is never extracted, so it must carry no text: any
    // text in one would ship unread.
    if sanitizer.role(kind) == PartRole::Structure
        && let Some(stray) = stray_text(raw)?
    {
        return Err(Error::sanitization(format!(
            "`{path}` carries text no extraction reads: `{stray}`"
        )));
    }
    // An embedding (an SVG image) is its own format, not Office XML.
    if matches!(kind, PartKind::Embedding(_) | PartKind::Font) {
        return Ok(());
    }
    // A part rebuilt from its schema holds only the schema's namespaces:
    // the fixed point above already vouches for it. What is left is the
    // property parts (custom XML never ships).
    if rebuilt(kind) {
        return Ok(());
    }
    let vocabulary = UnreviewedNamespaces::default();
    xml_edit::edit(raw, &vocabulary)?;
    if let Some(namespace) = vocabulary.unknown() {
        return Err(Error::sanitization(format!(
            "`{path}` uses the unreviewed XML namespace `{namespace}`"
        )));
    }
    Ok(())
}

/// The first non-blank text of `raw`, or `None` when the part carries none.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if `raw` is not well-formed XML.
fn stray_text(raw: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(raw);
    loop {
        let event = reader.read_event().map_err(malformed_xml)?;
        let text: &str = match &event {
            Event::Eof => return Ok(None),
            Event::Text(text) => text,
            Event::CData(text) => text,
            _ => continue,
        };
        let text = text.trim();
        if !text.is_empty() {
            return Ok(Some(text.chars().take(40).collect()));
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::redaction::{Disposition, HiddenContent};

    use super::*;
    use crate::docx::WordFormat;
    use crate::fixtures::{pack_parts, part_names, read_part};
    use crate::pptx::SlideFormat;
    use crate::xlsx::SheetFormat;

    const SAMPLE_DOCX: &[u8] = include_bytes!("../../testdata/docx/body_header_hyperlink.docx");
    const SAMPLE_PPTX: &[u8] = include_bytes!("../../testdata/pptx/slide_text.pptx");
    const SAMPLE_XLSX: &[u8] = include_bytes!("../../testdata/xlsx/shared_string_cells.xlsx");
    const SAMPLE2_XLSX: &[u8] = include_bytes!("../../testdata/xlsx/non_cell_text.xlsx");

    /// Sanitize `document`, then check the result passes: nothing more to
    /// remove, every reference resolved, every vocabulary reviewed.
    fn sanitizes_to_a_clean_package(document: &[u8], sanitizer: &dyn Sanitizer) {
        for policy in [
            HiddenContentPolicy::default(),
            HiddenContentPolicy::default()
                .with_metadata(Disposition::ScanAsContent)
                .with_collaboration(Disposition::ScanAsContent),
        ] {
            let sanitized = sanitize(document, policy, sanitizer).unwrap().bytes;
            let sanitized = sanitized.unwrap_or_else(|| document.to_vec());
            if let Err(e) = check(&sanitized, policy, sanitizer) {
                panic!("{policy:?}: {e}");
            }
        }
    }

    #[test]
    fn fixtures_sanitize_to_clean_packages() {
        sanitizes_to_a_clean_package(SAMPLE_DOCX, &WordFormat);
        sanitizes_to_a_clean_package(SAMPLE_PPTX, &SlideFormat);
        sanitizes_to_a_clean_package(SAMPLE_XLSX, &SheetFormat);
        sanitizes_to_a_clean_package(SAMPLE2_XLSX, &SheetFormat);
    }

    /// `package` with the part `name` rewritten by `edit`.
    fn replacing(package: &[u8], name: &str, edit: impl Fn(String) -> String) -> Vec<u8> {
        let parts: Vec<(String, Vec<u8>)> = part_names(package)
            .into_iter()
            .map(|part| {
                let bytes = read_part(package, &part).unwrap();
                let bytes = if part == name {
                    edit(String::from_utf8(bytes).unwrap()).into_bytes()
                } else {
                    bytes
                };
                (part, bytes)
            })
            .collect();
        let parts: Vec<(&str, &[u8])> = parts
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        pack_parts(&parts)
    }

    /// A property no field surfaces fails the check under `ScanAsContent`,
    /// since nothing could have detected what it holds.
    #[test]
    fn an_unsurfaced_property_fails_the_check() {
        let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
        let sanitized = sanitize(SAMPLE_DOCX, policy, &WordFormat).unwrap().bytes;
        let sanitized = replacing(&sanitized.unwrap(), "docProps/core.xml", |core| {
            core.replace(
                "</coreProperties>",
                "<revision>7</revision></coreProperties>",
            )
        });
        let error = check(&sanitized, policy, &WordFormat).unwrap_err();
        assert!(error.to_string().contains("Properties"), "{error}");
    }

    /// The fixture with vendor markup and text in its manifest and its
    /// document's relationships.
    fn with_vendor_package_markup() -> Vec<u8> {
        let package = replacing(SAMPLE_DOCX, "[Content_Types].xml", |manifest| {
            manifest
                .replace(
                    "<Types ",
                    r#"<?vendor note?><Types xmlns:v="urn:vendor" v:owner="Jane Roe" "#,
                )
                .replace(
                    "<Default Extension=\"xml\"",
                    "<v:note>Jane Roe</v:note>secret<Default v:by=\"Jane Roe\" Extension=\"xml\"",
                )
        });
        replacing(&package, "word/_rels/document.xml.rels", |rels| {
            rels.replace(
                "<Relationship Type=",
                "<Relationship Author=\"Jane Roe\" Type=",
            )
            .replace(
                "</Relationships>",
                "<Note>Jane Roe</Note><!--Jane Roe--></Relationships>",
            )
        })
    }

    #[test]
    fn package_parts_are_cut_to_their_vocabulary() {
        let document = with_vendor_package_markup();
        let policy = HiddenContentPolicy::default();
        let Sanitized {
            bytes: sanitized,
            stripped,
        } = sanitize(&document, policy, &WordFormat).unwrap();
        let sanitized = sanitized.unwrap();
        for part in ["[Content_Types].xml", "word/_rels/document.xml.rels"] {
            let text = String::from_utf8(read_part(&sanitized, part).unwrap()).unwrap();
            for gone in ["Jane Roe", "secret", "vendor note", "v:", "Note"] {
                assert!(!text.contains(gone), "{part} kept {gone}: {text}");
            }
        }
        let recorded = |content, part: &str| {
            stripped.iter().any(|entry| {
                entry.content == content && entry.at.as_ref().is_some_and(|at| at.as_str() == part)
            })
        };
        assert!(recorded(
            HiddenContent::ApplicationData,
            "[Content_Types].xml"
        ));
        assert!(recorded(HiddenContent::Undisplayed, "[Content_Types].xml"));
        assert!(recorded(
            HiddenContent::ApplicationData,
            "word/_rels/document.xml.rels"
        ));
        check(&sanitized, policy, &WordFormat).unwrap();
    }

    #[test]
    fn clean_package_parts_ship_byte_for_byte() {
        let sanitized = sanitize(SAMPLE_DOCX, HiddenContentPolicy::default(), &WordFormat)
            .unwrap()
            .bytes
            .unwrap();
        assert_eq!(
            read_part(&sanitized, "word/_rels/header3.xml.rels"),
            read_part(SAMPLE_DOCX, "word/_rels/header3.xml.rels")
        );
    }

    #[test]
    fn a_vendor_attribute_in_a_package_part_fails_the_check() {
        let policy = HiddenContentPolicy::default();
        let sanitized = sanitize(SAMPLE_DOCX, policy, &WordFormat).unwrap().bytes;
        let sanitized = replacing(&sanitized.unwrap(), "_rels/.rels", |rels| {
            rels.replace("<Relationship ", "<Relationship Author=\"Jane Roe\" ")
        });
        let error = check(&sanitized, policy, &WordFormat).unwrap_err();
        assert!(error.to_string().contains("_rels/.rels"), "{error}");
    }

    /// A strict document's relationship names are checked like a
    /// transitional one's.
    #[test]
    fn a_dangling_strict_reference_fails_the_check() {
        let policy = HiddenContentPolicy::default();
        let sanitized = sanitize(SAMPLE_DOCX, policy, &WordFormat).unwrap().bytes;
        let sanitized = replacing(&sanitized.unwrap(), "word/document.xml", |document| {
            document.replace(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                "http://purl.oclc.org/ooxml/officeDocument/relationships",
            )
        });
        check(&sanitized, policy, &WordFormat).unwrap();
        let dangling = replacing(&sanitized, "word/_rels/document.xml.rels", |rels| {
            rels.replace("Id=\"R207b36bfad034c28\"", "Id=\"rIdGone\"")
        });
        let error = check(&dangling, policy, &WordFormat).unwrap_err();
        assert!(error.to_string().contains("R207b36bfad034c28"), "{error}");
    }

    /// Under `ScanAsContent` a property part keeps only what its fields read:
    /// an entry out of place, a heading of no text type, a field name inside
    /// a list, a variant in the core properties, and stray text all go, the
    /// result passes the check, and sanitizing it again changes nothing.
    #[test]
    fn a_property_part_keeps_only_what_its_fields_read() {
        let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
        let document = replacing(SAMPLE_DOCX, "docProps/app.xml", |_| {
            r#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">Stray Roe<Company>Acme</Company><TitlesOfParts><vt:lpstr>Jane Roe salary</vt:lpstr><Company>Secret</Company></TitlesOfParts><HeadingPairs><vt:vector size="2" baseType="variant"><vt:variant><vt:lpstr>Pages</vt:lpstr></vt:variant><vt:variant><vt:i4>Jane Roe</vt:i4></vt:variant></vt:vector></HeadingPairs></Properties>"#.to_owned()
        });
        let document = replacing(&document, "docProps/core.xml", |core| {
            core.replace(
                "</coreProperties>",
                r#"<vt:lpwstr xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">Jane Roe</vt:lpwstr></coreProperties>"#,
            )
        });
        let sanitized = sanitize(&document, policy, &WordFormat)
            .unwrap()
            .bytes
            .unwrap();
        for part in ["docProps/app.xml", "docProps/core.xml"] {
            let text = String::from_utf8(read_part(&sanitized, part).unwrap()).unwrap();
            for gone in ["Jane Roe", "Stray", "Secret"] {
                assert!(!text.contains(gone), "{part} kept {gone}: {text}");
            }
        }
        let app = String::from_utf8(read_part(&sanitized, "docProps/app.xml").unwrap()).unwrap();
        assert!(app.contains("Acme") && app.contains("Pages"), "{app}");
        check(&sanitized, policy, &WordFormat).unwrap();
        let again = sanitize(&sanitized, policy, &WordFormat).unwrap().bytes;
        assert!(again.is_none_or(|again| again == sanitized));
    }

    /// Under `ScanAsContent` a property part keeps only the attributes its
    /// structure needs, the result passes the check, and sanitizing it again
    /// changes nothing.
    #[test]
    fn a_property_part_keeps_only_the_attributes_it_needs() {
        let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
        let document = replacing(SAMPLE_DOCX, "docProps/core.xml", |core| {
            core.replace("<lastModifiedBy>", r#"<lastModifiedBy note="Jane Roe">"#)
                .replace(
                    "<coreProperties ",
                    r#"<coreProperties xmlns:v="urn:Jane-Roe" "#,
                )
        });
        let sanitized = sanitize(&document, policy, &WordFormat)
            .unwrap()
            .bytes
            .unwrap();
        let core = String::from_utf8(read_part(&sanitized, "docProps/core.xml").unwrap()).unwrap();
        assert!(!core.contains("Jane") && !core.contains("Roe"), "{core}");
        assert!(core.contains(r#"xsi:type="dcterms:W3CDTF""#), "{core}");
        check(&sanitized, policy, &WordFormat).unwrap();
        let again = sanitize(&sanitized, policy, &WordFormat).unwrap().bytes;
        assert!(again.is_none_or(|again| again == sanitized));
    }

    /// A strict extended-properties part keeps only the surfaced fields when
    /// metadata is scanned, and then passes the check.
    #[test]
    fn a_strict_property_part_is_redacted_and_checked() {
        let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
        let document = replacing(SAMPLE_DOCX, "docProps/app.xml", |_| {
            r#"<?xml version="1.0"?><Properties xmlns="http://purl.oclc.org/ooxml/officeDocument/extendedProperties" xmlns:vt="http://purl.oclc.org/ooxml/officeDocument/docPropsVTypes"><Company>Acme</Company><Application>Vendor Writer</Application></Properties>"#.to_owned()
        });
        let sanitized = sanitize(&document, policy, &WordFormat)
            .unwrap()
            .bytes
            .unwrap();
        let app = String::from_utf8(read_part(&sanitized, "docProps/app.xml").unwrap()).unwrap();
        assert!(app.contains("Acme"));
        assert!(!app.contains("Vendor Writer"));
        check(&sanitized, policy, &WordFormat).unwrap();
    }
}
