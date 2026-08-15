//! Extract + rewrite across every text-bearing DOCX part, plus embeddings.

use std::io::{Cursor, Read, Write};

use elide_docx::block::{PartReplacement, Replacement};
use elide_docx::part::{EmbeddingKind, PartKind, PartPath};
use elide_docx::{Docx, ErrorKind};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const CONTENT_TYPES: &str = r#"<?xml version="1.0"?><Types/>"#;
const RELS: &str = r#"<?xml version="1.0"?><Relationships/>"#;
const BODY_PART: &str = "word/document.xml";
const MEDIA_PART: &str = "word/media/image1.png";
const MEDIA: &[u8] = b"\x89PNG\r\n\x1a\n-not-a-real-image";

/// A `w:t`-run XML part carrying `text`.
fn text_part(text: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
    )
}

/// A minimal `.docx` built from `(path, bytes)` parts, always including the
/// required structure and body.
fn docx_with(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut put = |name: &str, bytes: &[u8]| {
        zip.start_file(name, opts).unwrap();
        zip.write_all(bytes).unwrap();
    };
    put("[Content_Types].xml", CONTENT_TYPES.as_bytes());
    put("_rels/.rels", RELS.as_bytes());
    for (name, bytes) in parts {
        put(name, bytes);
    }
    zip.finish().unwrap().into_inner()
}

/// A body-only `.docx` with an image, matching the earlier fixture.
fn sample_docx(body_text: &str) -> Vec<u8> {
    docx_with(&[
        (BODY_PART, text_part(body_text).as_bytes()),
        (MEDIA_PART, MEDIA),
    ])
}

fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut zip = ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut e = zip.by_index(i).unwrap();
            let name = e.name().to_owned();
            let mut buf = Vec::new();
            e.read_to_end(&mut buf).unwrap();
            (name, buf)
        })
        .collect()
}

fn part_text(bytes: &[u8], part: &str) -> String {
    entries(bytes)
        .into_iter()
        .find(|(n, _)| n == part)
        .map(|(_, b)| String::from_utf8(b).unwrap())
        .unwrap()
}

#[test]
fn extracts_text_from_every_text_bearing_part() {
    let docx = docx_with(&[
        (BODY_PART, text_part("Body: Alice").as_bytes()),
        ("word/header1.xml", text_part("Header: Bob").as_bytes()),
        ("word/footer1.xml", text_part("Footer: Carol").as_bytes()),
        ("word/footnotes.xml", text_part("Note: Dave").as_bytes()),
        ("word/comments.xml", text_part("Comment: Eve").as_bytes()),
        (MEDIA_PART, MEDIA),
    ]);
    let extraction = Docx::open(&docx).unwrap().extract();

    let has = |part: &str, needle: &str| {
        extraction
            .blocks
            .iter()
            .any(|b| b.part == PartPath::new(part) && b.text.contains(needle))
    };
    assert!(has(BODY_PART, "Body: Alice"));
    assert!(has("word/header1.xml", "Header: Bob"));
    assert!(has("word/footer1.xml", "Footer: Carol"));
    assert!(has("word/footnotes.xml", "Note: Dave"));
    assert!(has("word/comments.xml", "Comment: Eve"));

    assert_eq!(extraction.embeddings.len(), 1);
    assert_eq!(extraction.embeddings[0].kind, EmbeddingKind::Image);
    assert_eq!(extraction.embeddings[0].part, PartPath::new(MEDIA_PART));

    // A clean extraction has no issues: every text part parsed.
    assert!(extraction.issues.is_empty());
}

#[test]
fn a_corrupt_text_part_becomes_an_issue_not_a_failure() {
    use elide_docx::block::IssueKind;

    // A body that parses, plus a header that is not valid UTF-8.
    let docx = docx_with(&[
        (BODY_PART, text_part("Alice").as_bytes()),
        ("word/header1.xml", &[0xff, 0xfe, 0xff]),
    ]);
    let extraction = Docx::open(&docx).unwrap().extract();

    // The body still extracted; the header is reported as an issue.
    assert!(extraction.blocks.iter().any(|b| b.text == "Alice"));
    assert_eq!(extraction.issues.len(), 1);
    assert_eq!(extraction.issues[0].part, PartPath::new("word/header1.xml"));
    assert_eq!(extraction.issues[0].kind, IssueKind::NotUtf8);
}

#[test]
fn part_kind_classifies_from_path() {
    assert_eq!(PartPath::new(BODY_PART).kind(), PartKind::Body);
    assert_eq!(PartPath::new("word/header2.xml").kind(), PartKind::Header);
    assert_eq!(PartPath::new("word/footer1.xml").kind(), PartKind::Footer);
    assert_eq!(
        PartPath::new("word/footnotes.xml").kind(),
        PartKind::Footnotes
    );
    assert_eq!(
        PartPath::new("word/comments.xml").kind(),
        PartKind::Comments
    );
    assert_eq!(
        PartPath::new(MEDIA_PART).kind(),
        PartKind::Embedding(EmbeddingKind::Image)
    );
    assert_eq!(
        PartPath::new("docProps/core.xml").kind(),
        PartKind::Metadata
    );
    assert_eq!(PartPath::new("word/settings.xml").kind(), PartKind::Other);
    assert_eq!(
        PartPath::new("word/charts/chart1.xml").kind(),
        PartKind::Chart
    );
    assert_eq!(
        PartPath::new("word/diagrams/data1.xml").kind(),
        PartKind::Diagram
    );
    assert_eq!(
        PartPath::new("word/glossary/document.xml").kind(),
        PartKind::Glossary
    );
    assert_eq!(
        PartPath::new("word/glossary/header1.xml").kind(),
        PartKind::Glossary
    );
    assert!(PartKind::Chart.is_text());
    assert!(PartKind::Diagram.is_text());
}

#[test]
fn rewrite_redacts_across_parts_and_preserves_others() {
    let docx = docx_with(&[
        (BODY_PART, text_part("Alice").as_bytes()),
        ("word/header1.xml", text_part("Bob").as_bytes()),
        (MEDIA_PART, MEDIA),
    ]);
    let extraction = Docx::open(&docx).unwrap().extract();

    let replacements: Vec<Replacement> = extraction
        .blocks
        .iter()
        .filter(|b| b.text == "Alice" || b.text == "Bob")
        .map(|b| Replacement::for_block(b, "[NAME]"))
        .collect();
    assert_eq!(replacements.len(), 2);

    let out = Docx::open(&docx).unwrap().rewrite(&replacements).unwrap();

    let body = part_text(&out, BODY_PART);
    let header = part_text(&out, "word/header1.xml");
    assert!(
        body.contains("[NAME]") && !body.contains("Alice"),
        "body: {body}"
    );
    assert!(
        header.contains("[NAME]") && !header.contains("Bob"),
        "header: {header}"
    );
    assert_eq!(
        entries(&out)
            .into_iter()
            .find(|(n, _)| n == MEDIA_PART)
            .unwrap()
            .1,
        MEDIA
    );
}

#[test]
fn rewrite_with_parts_replaces_an_embedding() {
    let docx = sample_docx("Alice");
    let redacted = b"REDACTED".to_vec();
    let out = Docx::open(&docx)
        .unwrap()
        .rewrite_with_parts(
            &[],
            &[PartReplacement {
                part: PartPath::new(MEDIA_PART),
                bytes: redacted.clone(),
            }],
        )
        .unwrap();

    assert_eq!(
        entries(&out)
            .into_iter()
            .find(|(n, _)| n == MEDIA_PART)
            .unwrap()
            .1,
        redacted
    );
}

#[test]
fn empty_rewrite_repacks_unchanged() {
    let docx = sample_docx("Alice");
    let out = Docx::open(&docx).unwrap().rewrite(&[]).unwrap();
    assert!(part_text(&out, BODY_PART).contains("Alice"));
}

#[test]
fn rewrite_is_fail_closed_on_overlap() {
    let docx = sample_docx("Alice");
    let body = PartPath::new(BODY_PART);
    let err = Docx::open(&docx)
        .unwrap()
        .rewrite(&[
            Replacement {
                part: body.clone(),
                start: 0,
                end: 10,
                text: "x".into(),
            },
            Replacement {
                part: body.clone(),
                start: 5,
                end: 15,
                text: "y".into(),
            },
        ])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
}

#[test]
fn rewrite_escapes_xml_metacharacters_and_reopens() {
    // A replacement whose text carries XML metacharacters must be escaped so the
    // rewritten package is still well-formed and re-opens.
    let docx = sample_docx("Alice");
    let extraction = Docx::open(&docx).unwrap().extract();
    let block = extraction
        .blocks
        .iter()
        .find(|b| b.text == "Alice")
        .unwrap();
    let replacement = Replacement::for_block(block, "<x> & <y>");

    let out = Docx::open(&docx).unwrap().rewrite(&[replacement]).unwrap();

    // The escaped text is in the body, the raw metacharacters are not, and the
    // package re-opens and parses.
    let body = part_text(&out, BODY_PART);
    assert!(body.contains("&lt;x&gt; &amp; &lt;y&gt;"), "body: {body}");
    assert!(!body.contains("<x>"), "body: {body}");
    let reopened = Docx::open(&out).unwrap().extract();
    assert!(reopened.issues.is_empty(), "issues: {:?}", reopened.issues);
    assert!(reopened.blocks.iter().any(|b| b.text == "<x> & <y>"));
}

#[test]
fn extraction_decodes_entities_and_round_trips() {
    // Entity-encoded source text is surfaced decoded, and a round-trip rewrite
    // re-opens and parses.
    let docx = sample_docx("Alice &amp; Bob");
    let extraction = Docx::open(&docx).unwrap().extract();
    assert!(
        extraction.blocks.iter().any(|b| b.text == "Alice & Bob"),
        "blocks: {:?}",
        extraction.blocks
    );

    let block = extraction
        .blocks
        .iter()
        .find(|b| b.text == "Alice & Bob")
        .unwrap();
    let out = Docx::open(&docx)
        .unwrap()
        .rewrite(&[Replacement::for_block(block, "[NAME]")])
        .unwrap();
    let reopened = Docx::open(&out).unwrap().extract();
    assert!(reopened.issues.is_empty(), "issues: {:?}", reopened.issues);
    assert!(reopened.blocks.iter().any(|b| b.text == "[NAME]"));
}

#[test]
fn rewrite_is_fail_closed_on_broken_comment_framing() {
    // A replacement spliced into a comment that would break `<!-- -->` framing
    // is refused rather than emitted as broken XML.
    let body = format!(
        r#"<?xml version="1.0"?><w:document><w:body><!-- {} --></w:body></w:document>"#,
        "redact-me"
    );
    let docx = docx_with(&[(BODY_PART, body.as_bytes())]);
    let extraction = Docx::open(&docx).unwrap().extract();
    let block = extraction
        .blocks
        .iter()
        .find(|b| b.text.contains("redact-me"))
        .unwrap();

    let err = Docx::open(&docx)
        .unwrap()
        .rewrite(&[Replacement::for_block(block, "a--b")])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
}

#[test]
fn rewrite_is_fail_closed_on_unknown_part() {
    let docx = sample_docx("Alice");
    let err = Docx::open(&docx)
        .unwrap()
        .rewrite(&[Replacement {
            part: PartPath::new("word/nope.xml"),
            start: 0,
            end: 1,
            text: "x".into(),
        }])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
}

#[test]
fn rewrite_rejects_targeting_a_binary_part_as_text() {
    let docx = sample_docx("Alice");
    let err = Docx::open(&docx)
        .unwrap()
        .rewrite(&[Replacement {
            part: PartPath::new(MEDIA_PART),
            start: 0,
            end: 1,
            text: "x".into(),
        }])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
}

#[test]
fn part_replacement_rejects_protected_part() {
    // A binary replacement must not clobber the body or content-types manifest.
    let docx = sample_docx("Alice");
    for protected in [BODY_PART, "[Content_Types].xml"] {
        let err = Docx::open(&docx)
            .unwrap()
            .rewrite_with_parts(
                &[],
                &[PartReplacement {
                    part: PartPath::new(protected),
                    bytes: b"junk".to_vec(),
                }],
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnsafeRewrite, "part: {protected}");
    }
}

#[test]
fn part_replacement_conflicting_with_text_splice_is_rejected() {
    // A text splice and a binary replacement on the same part is a conflicting
    // instruction and is refused.
    let docx = docx_with(&[
        (BODY_PART, text_part("Alice").as_bytes()),
        ("word/header1.xml", text_part("Bob").as_bytes()),
    ]);
    let extraction = Docx::open(&docx).unwrap().extract();
    let block = extraction.blocks.iter().find(|b| b.text == "Bob").unwrap();

    let err = Docx::open(&docx)
        .unwrap()
        .rewrite_with_parts(
            &[Replacement::for_block(block, "[NAME]")],
            &[PartReplacement {
                part: PartPath::new("word/header1.xml"),
                bytes: b"junk".to_vec(),
            }],
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
}

#[test]
fn not_a_zip_is_invalid_archive() {
    let err = Docx::open(b"this is not a zip").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArchive);
}

#[test]
fn missing_body_part_is_invalid_package() {
    let bytes = docx_with(&[("word/header1.xml", text_part("no body").as_bytes())]);
    let err = Docx::open(&bytes).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidPackage);
}
