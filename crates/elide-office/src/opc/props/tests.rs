use elide_core::entity::builtins;

use super::*;
use crate::fixtures::property_fields as pairs;
use crate::opc::props::strip::UnsurfacedProperties;
use crate::opc::props::vocabulary::UnreviewedNamespaces;
use crate::opc::props::walk::Walk;
use crate::opc::xml::edit::{Edit, edit, expect_edit};

const CORE: &[u8] = br#"<?xml version="1.0" encoding="utf-8"?>
<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
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

/// A document type declaration is refused before the root and inside an
/// element being removed alike.
#[test]
fn stripping_refuses_a_document_type() {
    let declared = r#"<!DOCTYPE d [<!ENTITY x "Jane">]><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"/>"#;
    assert!(UnsurfacedProperties.apply(declared).is_err());
    let nested = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"><cp:revision><!DOCTYPE d></cp:revision></cp:coreProperties>"#;
    assert!(UnsurfacedProperties.apply(nested).is_err());
    // Inside a heading count too, whose content is rewritten rather than read.
    let in_count = format!(
        r#"{EXTENDED}<HeadingPairs><vt:vector size="2" baseType="variant"><vt:variant><vt:lpstr>Pages</vt:lpstr></vt:variant><vt:variant><vt:i4>1<!DOCTYPE d></vt:i4></vt:variant></vt:vector></HeadingPairs></Properties>"#
    );
    assert!(UnsurfacedProperties.apply(&in_count).is_err());
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
    let read: Vec<(String, String)> = fields(xml.as_bytes())
        .into_iter()
        .map(|field| (field.key, field.value))
        .collect();
    let mut reader = NsReader::from_str(xml);
    let mut unread = Vec::new();
    let mut walk = Walk::default();
    // The field each open element holds, if any: a text is read only when
    // the field it sits in surfaces it.
    let mut open: Vec<Option<String>> = Vec::new();
    loop {
        let (resolved, event) = reader.read_resolved_event().unwrap();
        let namespace = bound(&resolved);
        match &event {
            Event::Start(e) => {
                let field = walk.enter(namespace.as_deref(), e);
                open.push(field.map(|field| field.key));
            }
            // An empty element still takes its place in a list.
            Event::Empty(e) => {
                walk.enter(namespace.as_deref(), e);
                walk.leave();
            }
            Event::End(_) => {
                walk.leave();
                open.pop();
            }
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
        let field = open.iter().rev().find_map(Option::as_deref);
        let surfaced = field.is_some_and(|field| {
            read.iter()
                .any(|(key, value)| key == field && value.contains(text))
        });
        if !text.is_empty() && !surfaced {
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
    let vocabulary = UnreviewedNamespaces::default();
    edit(&text, &vocabulary).unwrap();
    assert_eq!(vocabulary.unknown(), None, "{text}");
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
/// namespace is no field, and an element in an unreviewed namespace, or
/// in none, is neither read nor kept, even inside a field.
#[test]
fn a_field_is_matched_by_its_namespace() {
    let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:v="urn:vendor"><v:title>Jane Roe</v:title><cp:title>John Doe</cp:title><dc:title>Plan</dc:title><dc:subject>a<v:x>Ada</v:x>b</dc:subject><dc:description>c<note xmlns="">Jane</note>d</dc:description></cp:coreProperties>"#;
    assert_eq!(
        pairs(core.as_bytes()),
        [
            ("title".to_owned(), "Plan".to_owned()),
            ("subject".to_owned(), "ab".to_owned()),
            ("description".to_owned(), "cd".to_owned()),
        ]
    );
    let text = surfaced_only(core);
    assert_eq!(
        text,
        r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Plan</dc:title><dc:subject>ab</dc:subject><dc:description>cd</dc:description></cp:coreProperties>"#
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
    let app =
        format!("{EXTENDED}<TitlesOfParts><Company>Secret</Company></TitlesOfParts></Properties>");
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

/// An HTML entity resolves to nothing, as any entity XML does not define,
/// whatever quick-xml features the build unifies: it is neither read nor
/// kept.
#[test]
fn an_html_entity_is_read_as_nothing_and_removed() {
    let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jos&eacute; Roe</dc:creator></cp:coreProperties>"#;
    let read = pairs(core.as_bytes());
    assert_eq!(read, [("creator".to_owned(), "Jos Roe".to_owned())]);
    let text = surfaced_only(core);
    assert!(!text.contains("eacute"), "{text}");
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
        // An array-valued property goes whole.
        "Grid",
    ] {
        assert!(!text.contains(gone), "{gone} kept: {text}");
    }
    assert_eq!(
        text.matches(r#"fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}""#)
            .count(),
        2,
        "{text}"
    );
    for kept in [
        r#"pid="2" name="Client">"#,
        r#"pid="3" name="Team">"#,
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

    let core = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Ada</dc:creator><dc:creator>Alan</dc:creator></coreProperties>"#;
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
    let out = std::str::from_utf8(&out).unwrap();
    // The redacted timestamp goes whole: an empty one is not a date.
    assert!(!out.contains("created"), "{out}");
    assert!(!out.contains("never"), "{out}");
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
    let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator/></coreProperties>"#;
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
    let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator><![CDATA[Ada Lovelace]]></dc:creator></coreProperties>"#;
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
    let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Jos&#233; Bob</dc:creator></coreProperties>"#;
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
    let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>&#72;&#105;</dc:creator></coreProperties>"#;
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
    let xml = br#"<coreProperties xmlns="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>A &amp; B</dc:creator></coreProperties>"#;
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

/// Text repeating a surfaced value is unread outside the field that
/// surfaces it.
#[test]
fn a_repeated_value_outside_its_field_is_unread() {
    let app = format!("{EXTENDED}<Company>Ada</Company>Ada</Properties>");
    assert_eq!(unread(&app), ["Ada"]);
    let app = format!("{EXTENDED}<Company>Ada</Company></Properties>");
    assert_eq!(unread(&app), Vec::<String>::new());
}

/// A custom property whose value is an array goes whole, not left without a
/// typed value, and those after it are numbered on without a gap.
#[test]
fn a_custom_array_property_goes_whole() {
    let property = |pid: u32, name: &str, value: &str| {
        format!(
            r#"<property fmtid="{{D5CDD505-2E9C-101B-9397-08002B2CF9AE}}" pid="{pid}" name="{name}">{value}</property>"#
        )
    };
    let array = r#"<vt:array vt:lBounds="0" vt:uBounds="0" vt:baseType="lpwstr"><vt:lpwstr>Jane</vt:lpwstr></vt:array>"#;
    let empty_array = r#"<vt:array vt:lBounds="0" vt:uBounds="0" vt:baseType="lpwstr"/>"#;
    let kept = property(4, "Kept", "<vt:lpwstr>x</vt:lpwstr>");
    for value in [array, empty_array] {
        let xml = format!(
            "{CUSTOM_ROOT}>{}{kept}</Properties>",
            property(2, "Arr", value)
        );
        let out = UnsurfacedProperties.apply(&xml).unwrap().unwrap().text;
        assert!(!out.contains("Arr"), "{out}");
        assert!(!out.contains("array"), "{out}");
        assert!(
            out.contains(&property(2, "Kept", "<vt:lpwstr>x</vt:lpwstr>")),
            "{out}"
        );
        assert!(UnsurfacedProperties.apply(&out).unwrap().is_none(), "{out}");
    }
}
