use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

use super::*;
use crate::fixtures::{pack_typed, read_text_part};
use crate::opc::Replacement;
use crate::opc::check::check;
use crate::opc::sanitize::{Sanitized, sanitize};
use crate::text_package::OoxmlPackage;

/// The content type of a Word document's body.
const DOCUMENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
/// The content type of a Word document's comments.
const COMMENTS: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";

/// A Word document whose body carries a picture's description, a
/// hyperlink's tooltip, a content control's title and tag, a table caption,
/// and a tracked insertion, with a comments part beside it.
fn document() -> Vec<u8> {
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><w:body><w:p><w:hyperlink w:anchor="top" w:tooltip="Jane's page"><w:r><w:t>link</w:t></w:r></w:hyperlink><w:r><w:drawing><wp:inline><wp:docPr id="1" name="Picture 1" descr="C:\Users\jane\photo.jpg"/></wp:inline></w:drawing></w:r><w:ins w:id="1" w:author="Jane Roe" w:date="2024-01-02T03:04:05Z"><w:r><w:t>added</w:t></w:r></w:ins></w:p><w:sdt><w:sdtPr><w:alias w:val="Patient"/><w:tag w:val="patient-name"/></w:sdtPr><w:sdtContent><w:p/></w:sdtContent></w:sdt><w:tbl><w:tblPr><w:tblCaption w:val="Salaries of Jane Roe"/></w:tblPr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></w:body></w:document>"#
    );
    let comments = format!(
        r#"<?xml version="1.0"?><w:comments xmlns:w="{W}"><w:comment w:id="0" w:author="John Doe" w:initials="JD" w:date="2024-01-02T03:04:05Z"><w:p><w:r><w:t>check this</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    pack_typed(&[
            ("_rels/.rels", "", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#),
            ("word/document.xml", DOCUMENT, body.as_bytes()),
            ("word/comments.xml", COMMENTS, comments.as_bytes()),
        ])
}

/// A picture's payload, a VML picture bullet, and an SVG image survive
/// the schema rebuild.
#[test]
fn pictures_and_svg_images_are_kept() {
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:v="urn:schemas-microsoft-com:vml"><w:body><w:p><w:r><w:drawing><wp:inline><wp:extent cx="1" cy="1"/><wp:docPr id="1" name="Picture 1"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="logo.svg"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId1"/></pic:blipFill><pic:spPr/></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r><w:r><w:pict><v:shape id="s" style="width:9pt"><v:imagedata r:id="rId1"/></v:shape></w:pict></w:r></w:p></w:body></w:document>"#
    );
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/logo.svg"/></Relationships>"#;
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text>Jane Roe</text></svg>"#;
    let package = pack_typed(&[
            ("_rels/.rels", "", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#),
            ("word/document.xml", DOCUMENT, body.as_bytes()),
            ("word/_rels/document.xml.rels", "", rels.as_bytes()),
            ("word/media/logo.svg", "image/svg+xml", svg.as_bytes()),
        ]);
    let Sanitized {
        bytes: out,
        stripped,
    } = sanitize(&package, HiddenContentPolicy::default(), &WordFormat).unwrap();
    let out = out.unwrap_or(package);
    let document = read_text_part(&out, "word/document.xml");
    assert!(
        document.contains(r#"<a:blip r:embed="rId1"/>"#),
        "{document}"
    );
    assert!(
        document.contains(r#"<v:imagedata r:id="rId1"/>"#),
        "{document}"
    );
    assert_eq!(read_text_part(&out, "word/media/logo.svg"), svg);
    assert!(
        stripped
            .iter()
            .all(|entry| entry.content != HiddenContent::ApplicationData),
        "{stripped:?}"
    );
    check(&out, HiddenContentPolicy::default(), &WordFormat).unwrap();
}

#[test]
fn a_redacted_bookmark_is_fitted_and_its_hyperlinks_follow() {
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W}"><w:body><w:p><w:bookmarkStart w:id="0" w:name="JaneRoe"/><w:r><w:t>x</w:t></w:r><w:bookmarkEnd w:id="0"/><w:hyperlink w:anchor="JaneRoe"><w:r><w:t>see</w:t></w:r></w:hyperlink></w:p></w:body></w:document>"#
    );
    let package = pack_typed(&[("word/document.xml", DOCUMENT, body.as_bytes())]);
    let package = OoxmlPackage::<WordFormat>::open(&package).unwrap();
    let bookmark = body.find(r#"w:name="JaneRoe""#).unwrap() + r#"w:name=""#.len();
    let block = package
        .extract()
        .blocks
        .into_iter()
        .find(|block| block.span.start == bookmark)
        .expect("the bookmark name is surfaced");
    let out = package
        .rewrite_with_parts(
            &[Replacement {
                part: block.part.clone(),
                span: block.span.clone(),
                text: "[PERSON]".into(),
            }],
            &[],
        )
        .unwrap();
    let body = read_text_part(&out, "word/document.xml");
    assert!(body.contains(r#"w:name="PERSON_""#), "{body}");
    assert!(body.contains(r#"w:anchor="PERSON_""#), "{body}");
}

#[test]
fn hidden_labels_and_alternative_text_are_stripped() {
    let Sanitized {
        bytes: out,
        stripped,
    } = sanitize(&document(), HiddenContentPolicy::default(), &WordFormat).unwrap();
    let out = out.unwrap();
    let body = read_text_part(&out, "word/document.xml");
    for hidden in [
        "photo.jpg",
        "Jane's page",
        "Patient",
        "patient-name",
        "Salaries",
    ] {
        assert!(!body.contains(hidden), "{hidden} in {body}");
    }
    let count = |content| {
        stripped
            .iter()
            .filter(|entry| entry.content == content)
            .map(|entry| entry.count)
            .sum::<usize>()
    };
    assert_eq!(count(HiddenContent::AlternativeText), 3);
    assert_eq!(count(HiddenContent::FormField), 2);
    check(&out, HiddenContentPolicy::default(), &WordFormat).unwrap();
}

#[test]
fn redacted_collaboration_surfaces_its_authors_and_drops_its_dates() {
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
    let Sanitized { bytes: out, .. } = sanitize(&document(), policy, &WordFormat).unwrap();
    let out = out.unwrap();
    for name in ["word/document.xml", "word/comments.xml"] {
        assert!(
            !read_text_part(&out, name).contains("w:date="),
            "a date in {name}"
        );
    }
    let package = OoxmlPackage::<WordFormat>::open(&out).unwrap();
    let texts = package.extract().texts();
    for author in ["Jane Roe", "John Doe", "JD"] {
        assert!(
            texts.iter().any(|text| text == author),
            "{author} not in {texts:?}"
        );
    }
    check(&out, policy, &WordFormat).unwrap();
}

/// The strict wordprocessing namespace.
const STRICT_W: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";

/// A strict document's tracked revisions and comment anchors are resolved
/// as a transitional one's.
#[test]
fn strict_revisions_are_resolved() {
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{STRICT_W}"><w:body><w:p><w:r><w:t>kept</w:t></w:r><w:del w:id="1" w:author="Jane Roe" w:date="2024-01-02T03:04:05Z"><w:r><w:delText>deleted</w:delText></w:r></w:del><w:ins w:id="2" w:author="Jane Roe"><w:r><w:t>added</w:t></w:r></w:ins><w:commentRangeStart w:id="0"/></w:p></w:body></w:document>"#
    );
    let package = pack_typed(&[("word/document.xml", DOCUMENT, body.as_bytes())]);
    let policy = HiddenContentPolicy::default();
    let Sanitized { bytes: out, .. } = sanitize(&package, policy, &WordFormat).unwrap();
    let out = out.unwrap();
    let body = read_text_part(&out, "word/document.xml");
    for hidden in ["deleted", "w:del ", "w:ins ", "commentRangeStart", "w:date"] {
        assert!(!body.contains(hidden), "{hidden} in {body}");
    }
    assert!(body.contains("added"), "{body}");
    assert!(body.contains(STRICT_W), "{body}");
    check(&out, policy, &WordFormat).unwrap();
}

/// An accepted insertion's children keep the namespaces it declared, and
/// its siblings are left as they were.
#[test]
fn an_accepted_insertion_hands_its_namespaces_to_its_children() {
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{STRICT_W}"><w:body><w:p><w:ins w:id="2" xmlns:s="{STRICT_W}"><s:r><s:t>added</s:t></s:r><s:r xmlns:s="{STRICT_W}"><s:t>again</s:t></s:r></w:ins><w:r><w:t>kept</w:t></w:r></w:p></w:body></w:document>"#
    );
    let package = pack_typed(&[("word/document.xml", DOCUMENT, body.as_bytes())]);
    let policy = HiddenContentPolicy::default();
    let Sanitized { bytes: out, .. } = sanitize(&package, policy, &WordFormat).unwrap();
    let out = out.unwrap();
    let body = read_text_part(&out, "word/document.xml");
    assert!(
            body.contains(&format!(
                r#"<w:p><s:r xmlns:s="{STRICT_W}"><s:t>added</s:t></s:r><s:r xmlns:s="{STRICT_W}"><s:t>again</s:t></s:r><w:r><w:t>kept</w:t></w:r></w:p>"#
            )),
            "{body}"
        );
    check(&out, policy, &WordFormat).unwrap();
    let Sanitized { bytes: again, .. } = sanitize(&out, policy, &WordFormat).unwrap();
    assert!(again.is_none_or(|again| again == out));
}

/// A strict external hyperlink is visible content, kept with its text.
#[test]
fn a_strict_external_hyperlink_is_kept() {
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{STRICT_W}" xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships"><w:body><w:p><w:hyperlink r:id="rId1"><w:r><w:t>visit</w:t></w:r></w:hyperlink></w:p></w:body></w:document>"#
    );
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/></Relationships>"#;
    let package = pack_typed(&[
        ("word/document.xml", DOCUMENT, body.as_bytes()),
        ("word/_rels/document.xml.rels", "", rels.as_bytes()),
    ]);
    let policy = HiddenContentPolicy::default();
    let Sanitized {
        bytes: out,
        stripped,
    } = sanitize(&package, policy, &WordFormat).unwrap();
    let out = out.unwrap_or(package);
    let body = read_text_part(&out, "word/document.xml");
    assert!(body.contains(r#"<w:hyperlink r:id="rId1">"#), "{body}");
    assert!(
        read_text_part(&out, "word/_rels/document.xml.rels").contains("example.com"),
        "the hyperlink's relationship is kept"
    );
    assert!(
        stripped
            .iter()
            .all(|entry| entry.content != HiddenContent::Link),
        "{stripped:?}"
    );
    check(&out, policy, &WordFormat).unwrap();
}

#[test]
fn collaboration_blocks_are_flagged_by_what_they_are() {
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>kept</w:t></w:r><w:del w:id="2" w:author="Jane Roe"><w:r><w:delText>gone</w:delText></w:r></w:del></w:p></w:body></w:document>"#
    );
    let comments = format!(
        r#"<?xml version="1.0"?><w:comments xmlns:w="{W}"><w:comment w:id="0" w:author="John Doe" w:initials="JD"><w:p><w:r><w:t>check this</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let package = pack_typed(&[
        ("word/document.xml", DOCUMENT, body.as_bytes()),
        ("word/comments.xml", COMMENTS, comments.as_bytes()),
    ]);
    let package = OoxmlPackage::<WordFormat>::open(&package).unwrap();
    let flags: Vec<(String, Option<String>)> = package
        .extract()
        .blocks
        .into_iter()
        .map(|block| {
            let label = block.flag.map(|flag| flag.label.as_str().to_owned());
            (block.text.to_string(), label)
        })
        .collect();
    let flag = |text: &str| {
        flags
            .iter()
            .find(|(block, _)| block == text)
            .unwrap_or_else(|| panic!("{text} not in {flags:?}"))
            .1
            .clone()
    };
    assert_eq!(flag("kept"), None);
    assert_eq!(flag("gone").as_deref(), Some("communications_content"));
    assert_eq!(
        flag("check this").as_deref(),
        Some("communications_content")
    );
    for author in ["Jane Roe", "John Doe", "JD"] {
        assert_eq!(flag(author).as_deref(), Some("person_name"), "{author}");
    }
}
