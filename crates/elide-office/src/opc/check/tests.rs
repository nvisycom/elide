use elide_core::redaction::{Disposition, HiddenContent};

use super::*;
use crate::docx::WordFormat;
use crate::fixtures::{pack_parts, part_names, read_part};
use crate::pptx::SlideFormat;
use crate::xlsx::SheetFormat;

const SAMPLE_DOCX: &[u8] = include_bytes!("../../../testdata/docx/body_header_hyperlink.docx");
const SAMPLE_PPTX: &[u8] = include_bytes!("../../../testdata/pptx/slide_text.pptx");
const SAMPLE_XLSX: &[u8] = include_bytes!("../../../testdata/xlsx/shared_string_cells.xlsx");
const SAMPLE2_XLSX: &[u8] = include_bytes!("../../../testdata/xlsx/non_cell_text.xlsx");

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
/// Of two relationships parts that both dangle, the error names the first
/// in the package, every time.
#[test]
fn a_dangling_reference_names_the_first_relationships_part() {
    let policy = HiddenContentPolicy::default();
    let mut sanitized = sanitize(SAMPLE_DOCX, policy, &WordFormat)
        .unwrap()
        .bytes
        .unwrap();
    let rels = ["_rels/.rels", "word/_rels/document.xml.rels"];
    for part in rels {
        sanitized = replacing(&sanitized, part, |rels| {
            rels.replace(
                    "</Relationships>",
                    r#"<Relationship Id="rIdGone" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="/gone.png"/></Relationships>"#,
                )
        });
    }
    let names = crate::fixtures::part_names(&sanitized);
    let first = names
        .iter()
        .find(|name| rels.contains(&name.as_str()))
        .unwrap();
    for _ in 0..8 {
        let error = check(&sanitized, policy, &WordFormat).unwrap_err();
        assert!(error.to_string().contains(&format!("`{first}`")), "{error}");
    }
}

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

/// A reference is text to the stray-text check: one spelling a letter is
/// stray as plain text is, and one spelling whitespace is blank.
#[test]
fn a_referenced_letter_is_stray_text() {
    let root = r#"<volTypes xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><volType><main><tp><v>"#;
    let tail = "</v></tp></main></volType></volTypes>";
    let stray = |value: &str| stray_text(&format!("{root}{value}{tail}")).unwrap();
    assert_eq!(stray("Jane").as_deref(), Some("Jane"));
    assert_eq!(stray("&#74;&#97;").as_deref(), Some("&#74;"));
    assert_eq!(stray("&custom;").as_deref(), Some("&custom;"));
    assert_eq!(stray("&#32;&#x9;"), None);
}
