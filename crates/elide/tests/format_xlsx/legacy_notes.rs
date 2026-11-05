//! A legacy comment's note: the VML shape Excel draws it with, laid out as Excel
//! writes it, with a root and text markup in no namespace.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::modality::text::Text;
use elide::redaction::{Anonymizer, Disposition, HiddenContentPolicy};
use elide::{Directives, Document, Orchestrator};
use elide_office::fixtures::{pack_parts, read_part};

const VML: &str = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype><v:shape id="_x0000_s1025" type="#_x0000_t202" style='position:absolute;visibility:hidden' fillcolor="#ffffe1" o:insetmode="auto"><v:fill color2="#ffffe1"/><v:shadow color="black" obscured="t"/><v:path o:connecttype="none"/><v:textbox style='mso-direction-alt:auto'><div style='text-align:left'>private note</div></v:textbox><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>1, 15, 0, 2, 3, 15, 4, 16</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>0</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>"##;

/// A workbook holding [`VML`].
fn workbook() -> bytes::Bytes {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let main = r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#;
    bytes::Bytes::from(pack_parts(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("xl/workbook.xml", main.as_bytes()),
        ("xl/drawings/vmlDrawing1.vml", VML.as_bytes()),
    ]))
}

/// The drawing of the workbook redacted, with no rules, under `collaboration`.
async fn drawing(collaboration: Disposition) -> String {
    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_collaboration(collaboration);
    let orchestrator = Orchestrator::new()
        .with_registry(registry.clone())
        .with_modality::<Text>(Analyzer::new(), Anonymizer::new());
    let handle = registry
        .decode(workbook(), "xlsx", policy)
        .await
        .expect("decode xlsx");
    let mut documents = [Document::new("book.xlsx", handle)];
    orchestrator
        .anonymize(&mut documents, &Directives::new())
        .await
        .expect("anonymize");
    let out = documents[0].document.encode().expect("encode").to_bytes();
    String::from_utf8(read_part(&out, "xl/drawings/vmlDrawing1.vml").expect("drawing")).unwrap()
}

#[tokio::test]
async fn the_note_goes_with_its_comment_by_default() {
    let vml = drawing(Disposition::KeepOnlyEssential).await;
    assert!(!vml.contains("ClientData"), "{vml}");
}

#[tokio::test]
async fn a_scanned_note_keeps_its_text() {
    let vml = drawing(Disposition::ScanAsContent).await;
    assert!(vml.contains("private note"), "{vml}");
}

#[tokio::test]
async fn a_flagged_note_keeps_its_shape_without_its_text() {
    let vml = drawing(Disposition::FlagEverything).await;
    assert!(!vml.contains("private note"), "{vml}");
    assert!(vml.contains(r#"ObjectType="Note""#), "{vml}");
}
