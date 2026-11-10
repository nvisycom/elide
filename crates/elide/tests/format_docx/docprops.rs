//! End-to-end through the public facade: a DOCX's document properties
//! (`docProps/core.xml`) are detected and stripped alongside the body text,
//! driven by an `Orchestrator` a caller assembles from the re-exported
//! vocabulary.
//!
//! The `docx/body_header_hyperlink.docx` fixture carries `lastModifiedBy = "Oleh Martsokha"` in its
//! core properties. Under a hidden-content policy that treats metadata as
//! content, the built-in metadata pipeline detects that labelled field and
//! erases it; the DOCX re-encode writes the cleared property part back into the
//! package, so the output no longer names the editor. Under the default policy
//! the property parts are removed whole at decode.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::{LabelCatalog, builtins};
use elide::modality::metadata::Metadata;
use elide::modality::text::Text;
use elide::recognition::Scope;
use elide::redaction::operators::Erase;
use elide::redaction::{Anonymizer, Disposition, HiddenContentPolicy, Rule};
use elide::{Document, Orchestrator};
use elide_office::fixtures::{pack_parts, part_names, property_fields, read_part};

use super::FIXTURE;
use crate::support::orchestrator::{build_analyzer, build_anonymizer};

/// The last-modified-by author the fixture carries in its core properties.
const EDITOR: &str = "Oleh Martsokha";

fn has_editor(docx: &[u8]) -> bool {
    read_part(docx, "docProps/core.xml")
        .map(|core| {
            elide_office::fixtures::property_fields(&core)
                .iter()
                .any(|(k, v)| k == "lastModifiedBy" && v == EDITOR)
        })
        .unwrap_or(false)
}

#[tokio::test]
async fn docx_core_properties_are_stripped_through_the_facade() {
    let original = bytes::Bytes::from_static(FIXTURE.source);
    assert!(has_editor(&original), "fixture should name the editor");

    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry.clone())
        // Body text: an erase pipeline (any detected text redacts; not the focus).
        .with_modality::<Text>(
            Analyzer::new(),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );

    let handle = registry
        .decode(original.clone(), "docx", policy)
        .await
        .expect("decode docx");
    let mut documents = [Document::new("body_header_hyperlink.docx", handle)];

    let analyzed = orchestrator.analyze(&mut documents).await.expect("analyze");
    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await
        .expect("anonymize");

    let out = documents[0].document.encode().expect("encode").to_bytes();
    assert!(!has_editor(&out), "editor survived the docProps strip");

    // The output must still be a valid DOCX package whose other parts survive:
    // the docProps edit must not corrupt the archive or drop the body.
    assert!(
        read_part(&out, "word/document.xml").is_some(),
        "body part missing after docProps strip"
    );
    // The part itself survives, but a property no field surfaces (the
    // revision count) is stripped at decode: nothing could have detected what
    // it holds.
    let core = read_part(&out, "docProps/core.xml").expect("core.xml survives");
    assert!(
        !std::str::from_utf8(&core).unwrap().contains("revision>"),
        "an unsurfaced core property shipped"
    );
}

#[tokio::test]
async fn docx_core_properties_are_removed_by_default() {
    let registry = FormatRegistry::with_builtin();
    let document = registry
        .decode(
            bytes::Bytes::from_static(FIXTURE.source),
            "docx",
            HiddenContentPolicy::default(),
        )
        .await
        .expect("decode docx");
    let out = document.encode().expect("encode").to_bytes();
    assert!(
        read_part(&out, "docProps/core.xml").is_none(),
        "core.xml survived"
    );
    assert!(
        read_part(&out, "word/document.xml").is_some(),
        "body part missing"
    );
}

/// The fixture with a free-text description, a list of its parts' titles, and
/// custom properties: one text, one a number.
fn with_free_text_properties() -> bytes::Bytes {
    let edit = |name: &str, bytes: Vec<u8>| -> Vec<u8> {
        let text = String::from_utf8(bytes).expect("utf-8 part");
        let text = match name {
            "docProps/core.xml" => text.replace(
                "<dc:description></dc:description>",
                "<dc:description>Ask bob@example.com for the figures</dc:description>",
            ),
            "[Content_Types].xml" => text.replace(
                "</Types>",
                r#"<Override PartName="/docProps/custom.xml" ContentType="application/vnd.openxmlformats-officedocument.custom-properties+xml"/></Types>"#,
            ),
            "_rels/.rels" => text.replace(
                "</Relationships>",
                r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties" Target="docProps/custom.xml"/></Relationships>"#,
            ),
            _ => return text.into_bytes(),
        };
        text.into_bytes()
    };
    let mut parts: Vec<(String, Vec<u8>)> = part_names(FIXTURE.source)
        .into_iter()
        .map(|name| {
            let bytes = read_part(FIXTURE.source, &name).expect("part");
            let bytes = edit(&name, bytes);
            (name, bytes)
        })
        .collect();
    parts.push((
        "docProps/custom.xml".to_owned(),
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Reviewer"><vt:lpwstr>carol@example.com</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="Budget"><vt:i4>42</vt:i4></property></Properties>"#
            .to_vec(),
    ));
    let parts: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    bytes::Bytes::from(pack_parts(&parts))
}

/// The `(key, value)` fields of the property part `name` in `docx`.
fn fields(docx: &[u8], name: &str) -> Vec<(String, String)> {
    read_part(docx, name)
        .map(|xml| property_fields(&xml))
        .unwrap_or_default()
}

/// A recognizer reads property values: what it finds in a description or a
/// custom property is redacted and the rest ships, while a field it finds
/// nothing in, and whose label the catalog does not declare, is kept.
#[tokio::test]
async fn findings_inside_property_values_are_redacted_alone() {
    let registry = FormatRegistry::with_builtin();
    let catalog = LabelCatalog::from_iter([builtins::EMAIL_ADDRESS.clone()]);
    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(catalog))
        .with_registry(registry.clone())
        .with_modality::<Metadata>(
            build_analyzer::<Metadata>().expect("analyzer"),
            build_anonymizer::<Metadata>(),
        );
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let handle = registry
        .decode(with_free_text_properties(), "docx", policy)
        .await
        .expect("decode docx");
    let mut documents = [Document::new("report.docx", handle)];
    orchestrator
        .anonymize(&mut documents)
        .await
        .expect("anonymize");
    let out = documents[0].document.encode().expect("encode").to_bytes();

    let core = fields(&out, "docProps/core.xml");
    assert!(
        core.contains(&(
            "description".to_owned(),
            "Ask [email_address] for the figures".to_owned()
        )),
        "{core:?}"
    );
    assert!(core.contains(&("lastModifiedBy".to_owned(), EDITOR.to_owned())));
    assert_eq!(
        fields(&out, "docProps/custom.xml"),
        [
            ("property/0/name".to_owned(), "Reviewer".to_owned()),
            ("property/0".to_owned(), "[email_address]".to_owned()),
            ("property/1/name".to_owned(), "Budget".to_owned()),
            ("property/1".to_owned(), "42".to_owned()),
        ]
    );
}

/// Under the default policy a custom property part is removed like the rest.
#[tokio::test]
async fn custom_properties_are_removed_by_default() {
    let registry = FormatRegistry::with_builtin();
    let document = registry
        .decode(
            with_free_text_properties(),
            "docx",
            HiddenContentPolicy::default(),
        )
        .await
        .expect("decode docx");
    let out = document.encode().expect("encode").to_bytes();
    assert!(read_part(&out, "docProps/custom.xml").is_none());
}
