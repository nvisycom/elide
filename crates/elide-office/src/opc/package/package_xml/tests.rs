use super::*;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="main"/></Types>"#;

const RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t/styles" Target="styles.xml"/><Relationship Id="rId2" Type="t/link" Target="https://example.com" TargetMode="External"/></Relationships>"#;

/// The entries of the manifest `xml`, each as its name and its attributes'
/// keys and values.
fn read(xml: &str) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    entries(xml, &CONTENT_TYPES)
        .unwrap()
        .into_iter()
        .map(|entry| {
            let attributes = entry
                .attributes
                .into_iter()
                .map(|attr| (attr.key, attr.value))
                .collect();
            (entry.name, attributes)
        })
        .collect()
}

/// `raw` with the markup edit, then the text edit, applied.
fn allowlisted(raw: &str, vocabulary: &'static Vocabulary) -> String {
    let marked = PackageMarkup(vocabulary)
        .apply(raw)
        .unwrap()
        .map_or_else(|| raw.to_owned(), |edited| edited.text);
    PackageText
        .apply(&marked)
        .unwrap()
        .map_or(marked, |edited| edited.text)
}

#[test]
fn clean_parts_are_unchanged() {
    assert!(
        PackageMarkup(&CONTENT_TYPES)
            .apply(MANIFEST)
            .unwrap()
            .is_none()
    );
    assert!(PackageText.apply(MANIFEST).unwrap().is_none());
    assert!(PackageMarkup(&RELATIONSHIPS).apply(RELS).unwrap().is_none());
    assert!(PackageText.apply(RELS).unwrap().is_none());
}

#[test]
fn a_manifest_keeps_only_its_vocabulary() {
    let raw = r#"<?xml version="1.0"?><?vendor note?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types" xmlns:v="urn:vendor" v:owner="Jane"><v:note>secret</v:note><Default Extension="xml" ContentType="application/xml" v:by="Jane" Note="x"><Hidden/></Default>stray<Override PartName="/a.xml" ContentType="main"><![CDATA[cdata]]></Override><Other PartName="/b.xml"/>&amp;</Types>"#;
    let markup = PackageMarkup(&CONTENT_TYPES).apply(raw).unwrap().unwrap();
    assert_eq!(markup.count, 7);
    assert_eq!(
        allowlisted(raw, &CONTENT_TYPES),
        r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types" xmlns:v="urn:vendor"><Default Extension="xml" ContentType="application/xml"></Default><Override PartName="/a.xml" ContentType="main"></Override></Types>"#
    );
}

#[test]
fn a_relationships_part_keeps_only_its_vocabulary() {
    let raw = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t" Target="a.xml" Author="Jane"/><Extra Id="rId9"/>  <Relationship Id="rId2" Type="t" Target="b.xml">note</Relationship></Relationships>"#;
    assert_eq!(
        allowlisted(raw, &RELATIONSHIPS),
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t" Target="a.xml"/>  <Relationship Id="rId2" Type="t" Target="b.xml"></Relationship></Relationships>"#
    );
}

#[test]
fn an_entry_in_another_namespace_goes() {
    let raw = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:v="urn:vendor"><v:Relationship Id="rId1" Type="t" Target="a.xml"/></Relationships>"#;
    assert_eq!(
        allowlisted(raw, &RELATIONSHIPS),
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:v="urn:vendor"></Relationships>"#
    );
}

#[test]
fn an_allowlisted_part_is_a_fixed_point() {
    let raw = r#"<Types><Default Extension="xml" ContentType="x" Extra="1"/>text<x:y xmlns:x="urn:x"/></Types>"#;
    let once = allowlisted(raw, &CONTENT_TYPES);
    assert_eq!(allowlisted(&once, &CONTENT_TYPES), once);
}

/// A truncated part, one without a root, and one with a second root are
/// refused rather than cut down.
#[test]
fn a_part_that_is_not_one_whole_root_is_refused() {
    let truncated = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/>"#;
    assert!(PackageMarkup(&CONTENT_TYPES).apply(truncated).is_err());
    let open_entry = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t" Target="a.xml">"#;
    assert!(PackageMarkup(&RELATIONSHIPS).apply(open_entry).is_err());
    assert!(
        PackageMarkup(&CONTENT_TYPES)
            .apply("<?xml version=\"1.0\"?>")
            .is_err()
    );
    let twice = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
    assert!(PackageMarkup(&CONTENT_TYPES).apply(twice).is_err());
}

/// A document type declaration is refused before the root and inside an
/// entry being cut alike.
#[test]
fn a_document_type_is_refused() {
    let declared = r#"<!DOCTYPE Types [<!ENTITY x "Jane">]><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
    assert!(PackageMarkup(&CONTENT_TYPES).apply(declared).is_err());
    let nested = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Vendor><!DOCTYPE d></Vendor></Types>"#;
    assert!(PackageMarkup(&CONTENT_TYPES).apply(nested).is_err());
}

#[test]
fn a_foreign_root_is_refused() {
    assert!(
        PackageMarkup(&CONTENT_TYPES)
            .apply("<Relationships/>")
            .is_err()
    );
    let raw = r#"<Types xmlns="urn:vendor"/>"#;
    assert!(PackageMarkup(&CONTENT_TYPES).apply(raw).is_err());
}

/// The readers take only what the allowlist keeps: a foreign or nested
/// entry and a qualified attribute are not read.
#[test]
fn the_readers_take_only_what_the_allowlist_keeps() {
    let manifest = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types" xmlns:v="urn:vendor"><Default Extension="xml" ContentType="application/xml"/><v:Override PartName="/word/vendor.xml" ContentType="application/vnd.vendor"/><v:x><Override PartName="/word/nested.xml" ContentType="application/vnd.vendor"/></v:x><Override PartName="/word/document.xml" v:ContentType="application/vnd.vendor"/></Types>"#;
    let manifest_entries = read(manifest);
    assert_eq!(
        manifest_entries,
        [
            (
                "Default",
                vec![
                    ("Extension", "xml".to_owned()),
                    ("ContentType", "application/xml".to_owned())
                ]
            ),
            (
                "Override",
                vec![("PartName", "/word/document.xml".to_owned())]
            ),
        ]
    );
    let kept = allowlisted(manifest, &CONTENT_TYPES);
    assert_eq!(read(&kept), manifest_entries, "{kept}");
}

#[test]
fn vocabularies_follow_the_part_path() {
    assert!(std::ptr::eq(
        vocabulary_of(&PartPath::from(CONTENT_TYPES_PART)).unwrap(),
        &CONTENT_TYPES
    ));
    assert!(std::ptr::eq(
        vocabulary_of(&PartPath::from("word/_rels/document.xml.rels")).unwrap(),
        &RELATIONSHIPS
    ));
    assert!(vocabulary_of(&PartPath::from("word/document.xml")).is_none());
}
